//! First-person input: mouse look, movement, block targeting and interaction keys.

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use bevy_replicon::prelude::*;

use deepcore_shared::protocol::Intent;
use deepcore_shared::voxel::{raycast, BlockPos, Face};
use deepcore_shared::{GameContent, MovementParams};

use crate::world::ClientWorld;

/// The local player's camera.
#[derive(Component)]
pub struct PlayerCamera;

/// Local input state shared with prediction and the HUD.
#[derive(Resource, Default)]
pub struct InputState {
    /// Forward/right input, each -1..1.
    pub movement: [f32; 2],
    /// Look direction.
    pub yaw: f32,
    /// Look pitch.
    pub pitch: f32,
    /// Whether the left mouse button is held.
    pub mining: bool,
    /// Block under the crosshair.
    pub target: Option<BlockPos>,
    /// Face under the crosshair.
    pub target_face: Option<Face>,
    /// Whether the UI wants the mouse.
    pub ui_focus: bool,
    /// Whether the player is allowed to look around.
    pub captured: bool,
}

/// The input plugin.
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputState>()
            .add_systems(Startup, capture_cursor)
            .add_systems(
                Update,
                (
                    cursor_grab,
                    mouse_look,
                    keyboard_movement,
                    update_target,
                    mouse_actions,
                )
                    .chain(),
            );
    }
}

fn capture_cursor(mut cursors: Query<&mut CursorOptions>) {
    if let Ok(mut cursor) = cursors.single_mut() {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
}

fn cursor_grab(
    keys: Res<ButtonInput<KeyCode>>,
    mut cursors: Query<&mut CursorOptions>,
    mut input: ResMut<InputState>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        if let Ok(mut cursor) = cursors.single_mut() {
            let grabbed = cursor.grab_mode != CursorGrabMode::None;
            cursor.grab_mode = if grabbed {
                CursorGrabMode::None
            } else {
                CursorGrabMode::Locked
            };
            cursor.visible = grabbed;
            input.captured = !grabbed;
        }
    }
}

fn mouse_look(
    mut motion: MessageReader<MouseMotion>,
    mut input: ResMut<InputState>,
    camera: Query<&Transform, With<PlayerCamera>>,
) {
    if !input.captured {
        motion.clear();
        return;
    }
    let mut delta = Vec2::ZERO;
    for event in motion.read() {
        delta += event.delta;
    }
    input.yaw -= delta.x * 0.0025;
    input.pitch = (input.pitch - delta.y * 0.0025).clamp(-1.5533, 1.5533);
    let _ = camera;
}

fn keyboard_movement(keys: Res<ButtonInput<KeyCode>>, mut input: ResMut<InputState>) {
    let mut forward = 0.0;
    let mut right = 0.0;
    if keys.pressed(KeyCode::KeyW) {
        forward += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        forward -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        right += 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        right -= 1.0;
    }
    input.movement = [right, forward];
}

/// Casts a ray from the camera to find the targeted block.
fn update_target(
    mut input: ResMut<InputState>,
    world: Res<ClientWorld>,
    content: Res<GameContent>,
    params: Res<MovementParams>,
    cameras: Query<&Transform, With<PlayerCamera>>,
) {
    let Ok(transform) = cameras.single() else {
        return;
    };
    let origin = transform.translation.to_array();
    let dir = transform.forward().to_array();
    let reach = content.tuning.reach + 1.0;
    let hit = raycast(origin, dir, reach, |p| world.is_solid(&content, p));
    input.target = hit.map(|h| h.block);
    input.target_face = hit.map(|h| h.face);
    let _ = params;
}

fn mouse_actions(
    mouse: Res<ButtonInput<MouseButton>>,
    input: Res<InputState>,
    mut intents: MessageWriter<Intent>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if !input.captured {
        return;
    }
    if mouse.just_pressed(MouseButton::Left) {
        if let (Some(block), Some(face)) = (input.target, input.target_face) {
            intents.write(Intent::StartMining { block, face });
        }
    }
    if mouse.just_released(MouseButton::Left) {
        intents.write(Intent::StopMining);
    }
    if mouse.just_pressed(MouseButton::Right) {
        if let (Some(block), Some(face)) = (input.target, input.target_face) {
            intents.write(Intent::Interact { pos: block });
        }
    }
    // Number keys select hotbar slots.
    for (index, key) in [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ]
    .iter()
    .enumerate()
    {
        if keys.just_pressed(*key) {
            intents.write(Intent::SelectSlot { slot: index as u8 });
        }
    }
}
