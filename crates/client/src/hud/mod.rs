//! HUD: hotbar, crosshair, vitals, depth meter, Geiger counter and the debug overlay.

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::GameContent;

use crate::input::{InputState, PlayerCamera};
use crate::net::{ClientGameState, ClientNet};
use crate::prediction::Prediction;
use crate::render::Mesher;
use crate::world::ClientWorld;

/// The HUD plugin.
pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugOverlay>()
            .add_systems(Update, (toggle_debug, update_geiger));
    }
}

/// Debug overlay state toggled with F3.
#[derive(Resource, Default)]
pub struct DebugOverlay {
    /// Whether the overlay is visible.
    pub visible: bool,
}

/// Geiger counter click rate in clicks per second.
#[derive(Resource, Default)]
pub struct Geiger {
    /// Clicks per second.
    pub rate: f32,
    /// Accumulated click time.
    pub accumulator: f32,
}

fn toggle_debug(keys: Res<ButtonInput<KeyCode>>, mut overlay: ResMut<DebugOverlay>) {
    if keys.just_pressed(KeyCode::F3) {
        overlay.visible = !overlay.visible;
    }
}

/// The Geiger click rate follows the player's radiation dose.
fn update_geiger(
    state: Res<ClientGameState>,
    content: Res<GameContent>,
    mut geiger: ResMut<Geiger>,
    time: Res<Time>,
) {
    let dose = state.vitals.dose;
    let t = &content.tuning;
    geiger.rate = if dose < 1.0 {
        0.0
    } else {
        (dose / t.dose_damage * 12.0).clamp(0.5, 14.0)
    };
    if geiger.rate > 0.0 {
        geiger.accumulator += time.delta_secs();
    }
}

/// Depth in blocks below the surface, for the HUD.
pub fn depth_of(content: &GameContent, prediction: &Prediction) -> u32 {
    let y = prediction.state.position[1].max(0.0) as u32;
    content.tuning.surface_y.saturating_sub(y)
}

/// The name of the layer the player is in.
pub fn layer_name<'a>(content: &'a GameContent, prediction: &Prediction) -> &'a str {
    let y = prediction.state.position[1].max(0.0) as u32;
    content
        .layer_at_y(y)
        .map(|l| l.name.as_str())
        .unwrap_or("Surface")
}

/// HUD drawing data assembled each frame for the egui pass.
pub struct HudSnapshot {
    /// Health 0..100.
    pub health: f32,
    /// Oxygen 0..100.
    pub oxygen: f32,
    /// Radiation dose.
    pub dose: f32,
    /// Depth in blocks.
    pub depth: u32,
    /// Layer name.
    pub layer: String,
    /// Geiger clicks per second.
    pub geiger: f32,
    /// Whether the crosshair should be red (block too hard).
    pub too_hard: bool,
    /// Why the block is too hard.
    pub too_hard_text: Option<String>,
}

/// Builds the HUD snapshot from the current state.
pub fn snapshot(
    content: &GameContent,
    state: &ClientGameState,
    prediction: &Prediction,
    world: &ClientWorld,
    input: &InputState,
    geiger: &Geiger,
) -> HudSnapshot {
    let mut too_hard = false;
    let mut too_hard_text = None;
    if let Some(target) = input.target {
        let block = world.block(target);
        let def = &content.block(block).def;
        let tier = state
            .inventory
            .selected_stack()
            .map(|s| match &content.item(s.item).def.category {
                deepcore_shared::content::defs::ItemCategory::Tool(t) => t.tier,
                _ => 0,
            })
            .unwrap_or(0);
        if tier < def.min_tier {
            too_hard = true;
            too_hard_text = Some(format!("Too hard — needs Tier {}", def.min_tier));
        }
    }
    HudSnapshot {
        health: state.vitals.health,
        oxygen: state.vitals.oxygen,
        dose: state.vitals.dose,
        depth: depth_of(content, prediction),
        layer: layer_name(content, prediction).to_string(),
        geiger: geiger.rate,
        too_hard,
        too_hard_text,
    }
}

/// Debug text lines for the F3 overlay.
pub fn debug_lines(
    net: &ClientNet,
    world: &ClientWorld,
    mesher: &Mesher,
    prediction: &Prediction,
    client_state: &State<ClientState>,
) -> Vec<String> {
    let mut lines = vec![
        format!("server: {}", net.server.as_deref().unwrap_or("-")),
        format!("state: {:?}", client_state.get()),
        format!("chunks loaded: {}", world.loaded()),
        format!("mesh jobs: {}", mesher.jobs.len()),
        format!(
            "prediction: seq {} pending {} corrections {}",
            prediction.seq,
            prediction.pending.len(),
            prediction.corrections
        ),
        format!(
            "position: {:.2} {:.2} {:.2}",
            prediction.state.position[0],
            prediction.state.position[1],
            prediction.state.position[2]
        ),
        format!("ping: {:.0} ms", net.ping_ms),
    ];
    if let Some(info) = &net.info {
        lines.push(format!("seed: {} tick: {}", info.seed, info.tick));
    }
    lines
}

/// Keeps the camera's far plane in sync with the view distance.
pub fn sync_camera_far(
    settings: Res<crate::world::ClientSettings>,
    mut cameras: Query<&mut Projection, With<PlayerCamera>>,
) {
    if let Ok(mut projection) = cameras.single_mut() {
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.far = (settings.view_distance as f32 + 2.0) * 32.0;
        }
    }
}
