//! Server entry point: wires plugins, parses arguments and runs the 30 Hz simulation.

mod net;
mod sim;
mod world;

use std::net::SocketAddr;

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::RepliconRenetPlugins;
use clap::Parser;

use deepcore_shared::{GameContent, MovementParams, ProtocolPlugin, SharedPlugin};

/// Headless Deepcore server.
#[derive(Debug, Parser)]
#[command(name = "deepcore_server", about = "Authoritative Deepcore server")]
pub struct Args {
    /// Address to bind the UDP socket to.
    #[arg(long, default_value = "0.0.0.0:5000")]
    pub bind: String,
    /// Content directory.
    #[arg(long, default_value = deepcore_shared::DEFAULT_CONTENT_DIR)]
    pub content: String,
    /// World seed.
    #[arg(long, default_value_t = 12345)]
    pub seed: u64,
    /// Number of headless bots to spawn for load testing.
    #[arg(long, default_value_t = 0)]
    pub bots: usize,
    /// Simulated latency in milliseconds applied to intents.
    #[arg(long, default_value_t = 0)]
    pub simulate_latency: u64,
    /// Simulated packet loss percentage.
    #[arg(long, default_value_t = 0)]
    pub simulate_loss: u8,
    /// Directory for save files.
    #[arg(long, default_value = "saves")]
    pub save_dir: String,
    /// Run only N ticks then exit (used by integration tests).
    #[arg(long, default_value_t = 0)]
    pub max_ticks: u64,
}

fn main() {
    let args = Args::parse();
    if let Err(e) = run(args) {
        eprintln!("deepcore_server: {e}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<(), String> {
    let (content, movement) =
        deepcore_shared::load_content_resources(&args.content).map_err(|e| e.to_string())?;
    tracing::info!(
        "loaded content: {} blocks, {} items, {} recipes, {} machines, hash {:016x}",
        content.blocks.len(),
        content.items.len(),
        content.recipes.len(),
        content.machines.len(),
        content.hash
    );

    let tick_hz = content.tuning.tick_hz;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(StatesPlugin)
        .add_plugins(bevy::log::LogPlugin::default())
        .insert_resource(Time::<Fixed>::from_hz(f64::from(tick_hz)))
        .add_plugins((SharedPlugin, RepliconPlugins, RepliconRenetPlugins, ProtocolPlugin))
        .insert_resource(content)
        .insert_resource(movement)
        .insert_resource(net::ServerNetConfig {
            bind_addr: args.bind.clone(),
            simulate_latency_ms: args.simulate_latency,
            simulate_loss_percent: args.simulate_loss,
        })
        .insert_resource(net::ServerOptions {
            seed: args.seed,
            bots: args.bots,
            max_ticks: args.max_ticks,
            save_dir: args.save_dir.clone(),
        });

    app.add_plugins((sim::SimPlugin, net::NetPlugin));

    let bind: SocketAddr = args
        .bind
        .parse()
        .map_err(|e| format!("invalid bind address '{}': {e}", args.bind))?;
    tracing::info!("binding UDP socket on {bind}");

    app.run();
    Ok(())
}
