//! Handshake: protocol and content-hash check, player spawning, disconnects.

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::protocol::{
    ClientHello, HandshakeRejected, ServerInfo, PROTOCOL_VERSION,
};
use deepcore_shared::GameContent;

use crate::sim::components::{player_bundle, ServerPlayer};
use crate::world::World;

/// Per-client handshake state.
#[derive(Component)]
pub struct ClientSession {
    /// Display name once the client has introduced itself.
    pub name: Option<String>,
    /// Whether the handshake succeeded.
    pub authorized: bool,
    /// Player entity spawned for this client.
    pub player: Option<Entity>,
}

/// The handshake plugin.
pub struct HandshakePlugin;

impl Plugin for HandshakePlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_client_connected)
            .add_systems(
                PreUpdate,
                (handle_hello, handle_disconnect).run_if(in_state(ServerState::Running)),
            );
    }
}

fn on_client_connected(
    add: On<Add<ConnectedClient>>,
    mut commands: Commands,
) {
    commands.entity(add.entity).insert((
        ClientSession { name: None, authorized: false, player: None },
        crate::net::stream::ClientInterest::default(),
    ));
}

/// Processes `ClientHello` messages, rejects mismatched clients and spawns players.
fn handle_hello(
    mut hellos: MessageReader<FromClient<ClientHello>>,
    mut commands: Commands,
    mut sessions: Query<&mut ClientSession>,
    content: Res<GameContent>,
    world: Res<World>,
    mut infos: MessageWriter<ToClients<ServerInfo>>,
    mut rejections: MessageWriter<ToClients<HandshakeRejected>>,
) {
    for FromClient { client_id, message } in hellos.read() {
        let Some(entity) = client_id.entity() else {
            continue;
        };
        let Ok(mut session) = sessions.get_mut(entity) else {
            continue;
        };
        if session.authorized {
            continue;
        }
        if message.protocol != PROTOCOL_VERSION {
            rejections.write(ToClients {
                targets: SendTargets::Single(*client_id),
                message: HandshakeRejected {
                    reason: format!(
                        "protocol mismatch: server {PROTOCOL_VERSION}, client {}",
                        message.protocol
                    ),
                },
            });
            commands.write_message(DisconnectRequest { client: entity });
            continue;
        }
        if message.content_hash != content.hash {
            rejections.write(ToClients {
                targets: SendTargets::Single(*client_id),
                message: HandshakeRejected {
                    reason: "Content mismatch".to_string(),
                },
            });
            commands.write_message(DisconnectRequest { client: entity });
            continue;
        }
        session.authorized = true;
        session.name = Some(message.name.clone());
        let spawn = world.generator.spawn_point();
        let mut player = ServerPlayer::new(*client_id, message.name.clone(), spawn);
        // The starting steel pickaxe lives in hotbar slot 0.
        if let Some(pickaxe) = content.item_id("steel_pickaxe") {
            player.inventory.slots[0] =
                deepcore_shared::inventory::Slot::with(pickaxe, 1);
            player.tool = Some(pickaxe);
        }
        let movement = deepcore_shared::movement::MovementState::at_spawn(spawn);
        let player_entity = commands.spawn(player_bundle(message.name.clone(), movement)).id();
        commands.entity(player_entity).insert(player);
        session.player = Some(player_entity);
        infos.write(ToClients {
            targets: SendTargets::Single(*client_id),
            message: ServerInfo {
                protocol: PROTOCOL_VERSION,
                content_hash: content.hash,
                seed: world.seed,
                tick: 0,
            },
        });
        info!("player '{}' joined from client {}", message.name, client_id);
    }
}

fn handle_disconnect(
    mut removals: RemovedComponents<ConnectedClient>,
    sessions: Query<&ClientSession>,
    mut commands: Commands,
    players: Query<&ServerPlayer>,
) {
    for entity in removals.read() {
        let Ok(session) = sessions.get(entity) else {
            continue;
        };
        if let Some(player) = session.player {
            if let Ok(server_player) = players.get(player) {
                info!("player '{}' disconnected", server_player.name);
            }
            commands.entity(player).despawn();
        }
    }
}
