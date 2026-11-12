//! Client networking: connect, handshake, receive authoritative messages.

use std::net::{SocketAddr, UdpSocket};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon::shared::backend::channels::RepliconChannels;
use bevy_replicon_renet::netcode::{ClientAuthentication, NetcodeClientTransport};
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::RenetClient;
use bevy_replicon_renet::RenetChannelsExt;

use deepcore_shared::protocol::{
    ChatLine, ClientHello, HandshakeRejected, Intent, InventoryUpdate, MachineContents,
    MovementAck, NetMovement, NetworkSnapshot, Notify, PROTOCOL_VERSION, ScannerResult,
    ServerInfo, TechState,
};
use deepcore_shared::{GameContent, Inventory, Vitals};

use crate::world::ClientSettings;

/// The client's network state.
#[derive(Resource, Default)]
pub struct ClientNet {
    /// Server address currently connected or connecting to.
    pub server: Option<String>,
    /// Whether the handshake completed.
    pub handshaked: bool,
    /// Rejection or error text for the menu.
    pub error: Option<String>,
    /// Latest server info.
    pub info: Option<ServerInfo>,
    /// Ping estimate in milliseconds.
    pub ping_ms: f32,
}

/// The client's mirrored game state.
#[derive(Resource, Default)]
pub struct ClientGameState {
    /// The local player's inventory.
    pub inventory: Inventory,
    /// The local player's vitals.
    pub vitals: Vitals,
    /// Research state.
    pub researched: Vec<deepcore_shared::content::ids::TechId>,
    /// Research progress.
    pub research_progress: Vec<(
        deepcore_shared::content::ids::TechId,
        Vec<(deepcore_shared::content::ids::ItemId, u16)>,
    )>,
    /// Machine UI contents, if a machine is open.
    pub open_machine: Option<MachineContents>,
    /// Recent notifications.
    pub notifications: Vec<(String, f32)>,
    /// Recent chat lines.
    pub chat: Vec<String>,
    /// Latest network snapshot.
    pub networks: Option<NetworkSnapshot>,
    /// Ore scanner results, with their expiry time.
    pub scan: Option<(Vec<deepcore_shared::BlockPos>, f32)>,
}

/// The client networking plugin.
pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientNet>()
            .init_resource::<ClientGameState>()
            .add_systems(Startup, auto_connect)
            .add_systems(
                Update,
                (
                    send_hello,
                    receive_server_info,
                    receive_rejection,
                    receive_inventory,
                    receive_machine,
                    receive_tech,
                    receive_notifications,
                    receive_chat,
                    receive_network,
                    receive_scan,
                    expire_notifications,
                ),
            );
    }
}

fn auto_connect(
    settings: Res<ClientSettings>,
    mut net: ResMut<ClientNet>,
    mut commands: Commands,
    content: Res<GameContent>,
    channels: Res<RepliconChannels>,
) {
    if let Some(address) = settings.initial_connect.clone() {
        if let Err(e) = connect(&mut commands, &address, &settings, &content, &channels) {
            error!("could not connect: {e}");
            net.error = Some(e);
        } else {
            info!("connecting to {address}");
            net.server = Some(address);
        }
    }
}

/// Connects to a server. Used by the main menu's "Join by IP".
pub fn connect(
    commands: &mut Commands,
    address: &str,
    settings: &ClientSettings,
    _content: &GameContent,
    channels: &RepliconChannels,
) -> Result<(), String> {
    let server_addr: SocketAddr = address
        .parse()
        .map_err(|e| format!("invalid address '{address}': {e}"))?;
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind failed: {e}"))?;
    // Netcode compares the client clock against the server's, which uses seconds since the epoch.
    // Netcode compares clocks between peers; the transport advances this value itself, so start
    // from zero on the client (the server uses an absolute timestamp only for its own bookkeeping).
    let current_time = std::time::Duration::ZERO;
    let client_id = rand_id();
    let auth = ClientAuthentication::Unsecure {
        protocol_id: u64::from(PROTOCOL_VERSION),
        client_id,
        server_addr,
        user_data: None,
    };
    let transport = NetcodeClientTransport::new(current_time, auth, socket)
        .map_err(|e| format!("transport failed: {e}"))?;
    let connection_config = ConnectionConfig {
        server_channels_config: channels.server_configs(),
        client_channels_config: channels.client_configs(),
        ..Default::default()
    };
    commands.insert_resource(transport);
    commands.insert_resource(RenetClient::new(connection_config));
    let _ = settings;
    Ok(())
}

fn rand_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

fn send_hello(
    mut net: ResMut<ClientNet>,
    settings: Res<ClientSettings>,
    content: Res<GameContent>,
    mut hellos: MessageWriter<ClientHello>,
    state: Res<State<ClientState>>,
    mut sent: Local<bool>,
) {
    if *state.get() != ClientState::Connected {
        *sent = false;
        return;
    }
    if *sent || net.handshaked {
        return;
    }
    hellos.write(ClientHello {
        protocol: PROTOCOL_VERSION,
        content_hash: content.hash,
        name: settings.name.clone(),
        token: None,
    });
    *sent = true;
}

fn receive_server_info(
    mut infos: MessageReader<ServerInfo>,
    mut net: ResMut<ClientNet>,
) {
    for info in infos.read() {
        net.info = Some(*info);
        net.handshaked = true;
        info!("connected to server, seed {}", info.seed);
    }
}

fn receive_rejection(
    mut rejections: MessageReader<HandshakeRejected>,
    mut net: ResMut<ClientNet>,
) {
    for rejection in rejections.read() {
        error!("connection refused: {}", rejection.reason);
        net.error = Some(rejection.reason.clone());
        net.handshaked = false;
    }
}

fn receive_inventory(
    mut updates: MessageReader<InventoryUpdate>,
    mut state: ResMut<ClientGameState>,
) {
    for update in updates.read() {
        state.inventory = update.inventory.clone();
        state.vitals = update.vitals;
    }
}

fn receive_machine(
    mut contents: MessageReader<MachineContents>,
    mut state: ResMut<ClientGameState>,
) {
    for message in contents.read() {
        state.open_machine = Some(message.clone());
    }
}

fn receive_tech(
    mut tech: MessageReader<TechState>,
    mut state: ResMut<ClientGameState>,
) {
    for message in tech.read() {
        state.researched = message.researched.clone();
        state.research_progress = message.progress.clone();
    }
}

fn receive_notifications(
    mut notifies: MessageReader<Notify>,
    mut state: ResMut<ClientGameState>,
) {
    for message in notifies.read() {
        state.notifications.push((message.text.clone(), 4.0));
        if state.notifications.len() > 6 {
            state.notifications.remove(0);
        }
    }
}

fn receive_chat(mut chat: MessageReader<ChatLine>, mut state: ResMut<ClientGameState>) {
    for line in chat.read() {
        let text = match &line.from {
            Some(from) => format!("{from}: {}", line.text),
            None => line.text.clone(),
        };
        state.chat.push(text);
        if state.chat.len() > 50 {
            state.chat.remove(0);
        }
    }
}

fn receive_network(
    mut snapshots: MessageReader<NetworkSnapshot>,
    mut state: ResMut<ClientGameState>,
) {
    for snapshot in snapshots.read() {
        state.networks = Some(snapshot.clone());
    }
}

fn receive_scan(
    mut scans: MessageReader<ScannerResult>,
    mut state: ResMut<ClientGameState>,
) {
    for scan in scans.read() {
        state.scan = Some((scan.positions.clone(), 20.0));
    }
}

fn expire_notifications(mut state: ResMut<ClientGameState>, time: Res<Time>) {
    let dt = time.delta_secs();
    state.notifications.retain_mut(|(_, ttl)| {
        *ttl -= dt;
        *ttl > 0.0
    });
    if let Some((_, ttl)) = &mut state.scan {
        *ttl -= dt;
        if *ttl <= 0.0 {
            state.scan = None;
        }
    }
}

/// The local player's replicated entity, once it arrives.
#[derive(Resource, Default)]
pub struct LocalPlayer {
    /// Our own replicated entity, if known.
    pub entity: Option<Entity>,
    /// Last replicated position, for the debug overlay.
    pub last_position: Option<[f32; 3]>,
}

/// Finds the local player entity by matching the replicated movement to our prediction.
/// Connects to the address typed into the menu.
pub fn connect_from_menu(
    commands: &mut Commands,
    address: &str,
    settings: &ClientSettings,
    content: &GameContent,
    channels: &RepliconChannels,
) -> Result<(), String> {
    connect(commands, address, settings, content, channels)
}

/// Tracks the local player entity once replication arrives.
pub fn track_local_player(
    mut local: ResMut<LocalPlayer>,
    players: Query<(Entity, &NetMovement)>,
) {
    if local.entity.is_some() {
        return;
    }
    if let Some((entity, movement)) = players.iter().next() {
        local.entity = Some(entity);
        local.last_position = Some(movement.0.position);
    }
}
