//! Networking plugin for the server: transport, handshake, chunk streaming, intent validation.

pub mod handshake;
pub mod stream;
pub mod transport;

use bevy::prelude::*;

use crate::net::handshake::HandshakePlugin;
use crate::net::stream::StreamPlugin;

/// Runtime options that come from the command line.
#[derive(Resource, Debug, Clone)]
pub struct ServerOptions {
    /// World seed.
    pub seed: u64,
    /// Number of bots to spawn.
    pub bots: usize,
    /// Stop after this many ticks (0 = run forever).
    pub max_ticks: u64,
    /// Save directory.
    pub save_dir: String,
}

/// Network tuning, including latency and loss simulation.
#[derive(Resource, Debug, Clone)]
pub struct ServerNetConfig {
    /// Address to bind.
    pub bind_addr: String,
    /// Extra delay applied to inbound intents.
    pub simulate_latency_ms: u64,
    /// Percentage of inbound intents dropped.
    pub simulate_loss_percent: u8,
}

/// The server's networking plugin.
pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((HandshakePlugin, StreamPlugin));
    }

    fn finish(&self, app: &mut App) {
        // Channels are assigned in registration order, so the server socket can only be created
        // once every message type has been registered.
        let config = app
            .world()
            .get_resource::<ServerNetConfig>()
            .cloned()
            .unwrap_or(ServerNetConfig {
                bind_addr: "0.0.0.0:5000".into(),
                simulate_latency_ms: 0,
                simulate_loss_percent: 0,
            });
        match transport::start_server(app, &config) {
            Ok(()) => tracing::info!("server listening on {}", config.bind_addr),
            Err(e) => {
                tracing::error!("failed to start server: {e}");
                std::process::exit(1);
            }
        }
    }
}
