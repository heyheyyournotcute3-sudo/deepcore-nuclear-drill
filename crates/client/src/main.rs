//! Deepcore client: windowed, predicted movement, voxel rendering and egui UI.

mod audio;
mod hud;
mod input;
mod net;
mod prediction;
mod render;
mod ui;
mod world;

use bevy::prelude::*;

use bevy_egui::EguiPlugin;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::RepliconRenetPlugins;
use clap::Parser;

use deepcore_shared::{ProtocolPlugin, SharedPlugin, DEFAULT_CONTENT_DIR};

/// Windowed Deepcore client.
#[derive(Debug, Parser)]
#[command(name = "deepcore_client", about = "Deepcore client")]
pub struct Args {
    /// Server address to join, e.g. `127.0.0.1:5000`. Omit to show the main menu.
    #[arg(long)]
    pub connect: Option<String>,
    /// Display name.
    #[arg(long, default_value = "Miner")]
    pub name: String,
    /// Content directory.
    #[arg(long, default_value = DEFAULT_CONTENT_DIR)]
    pub content: String,
    /// Simulated latency in milliseconds.
    #[arg(long, default_value_t = 0)]
    pub simulate_latency: u64,
    /// Simulated packet loss percentage.
    #[arg(long, default_value_t = 0)]
    pub simulate_loss: u8,
    /// Render distance in chunks.
    #[arg(long, default_value_t = 6)]
    pub view_distance: i32,
}

fn main() {
    let args = Args::parse();
    let (content, movement) = match deepcore_shared::load_content_resources(&args.content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("deepcore_client: content error: {e}");
            std::process::exit(1);
        }
    };

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Deepcore".into(),
                    resolution: (1280, 720).into(),
                    ..default()
                }),
                ..default()
            })
            .set(ImagePlugin::default_nearest()),
    )

    .add_plugins(EguiPlugin::default())
    .add_plugins((SharedPlugin, RepliconPlugins, RepliconRenetPlugins, ProtocolPlugin))
    .insert_resource(content)
    .insert_resource(movement)
    .insert_resource(world::ClientSettings {
        name: args.name.clone(),
        view_distance: args.view_distance,
        simulate_latency_ms: args.simulate_latency,
        simulate_loss_percent: args.simulate_loss,
        initial_connect: args.connect.clone(),
    });

    app.add_plugins((
        render::RenderPlugin,
        world::WorldPlugin,
        input::InputPlugin,
        prediction::PredictionPlugin,
        net::ClientNetPlugin,
        hud::HudPlugin,
        ui::UiPlugin,
        audio::AudioPlugin,
    ));

    app.run();
}
