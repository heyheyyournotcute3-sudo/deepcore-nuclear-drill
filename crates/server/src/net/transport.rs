//! Networking: renet transport, handshake, channels and per-client chunk interest.

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon::shared::backend::channels::RepliconChannels;
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::RenetChannelsExt;

use crate::net::ServerNetConfig;

/// Creates the renet server and inserts its resources.
pub fn start_server(app: &mut App, config: &ServerNetConfig) -> Result<(), String> {
    use bevy_replicon_renet::RenetServer;
    use bevy_replicon_renet::netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig};
    use std::net::UdpSocket;
    use std::time::{Duration, SystemTime};

    let channels = app.world().resource::<RepliconChannels>();
    let connection_config = ConnectionConfig {
        server_channels_config: channels.server_configs(),
        client_channels_config: channels.client_configs(),
        ..Default::default()
    };
    let socket = UdpSocket::bind(config.bind_addr.clone())
        .map_err(|e| format!("failed to bind {}: {e}", config.bind_addr))?;
    socket
        .set_nonblocking(true)
        .map_err(|e| format!("failed to set non-blocking: {e}"))?;

    // Netcode validates the client's timestamp against ours, so both sides must start from the
    // same time base. The transport advances this value itself once it is running.
    let server_config = ServerConfig {
        current_time: Duration::ZERO,
        max_clients: deepcore_shared::protocol::MAX_PLAYERS,
        protocol_id: u64::from(deepcore_shared::PROTOCOL_VERSION),
        public_addresses: Vec::new(),
        authentication: ServerAuthentication::Unsecure,
    };

    let server = RenetServer::new(connection_config);

    let transport = NetcodeServerTransport::new(server_config, socket)
        .map_err(|e| format!("failed to create server transport: {e}"))?;

    app.insert_resource(server);
    app.insert_resource(transport);
    Ok(())
}
