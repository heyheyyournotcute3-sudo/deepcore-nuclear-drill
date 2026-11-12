//! Chunk streaming, interest management and block-change broadcasting.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::protocol::{
    BlockChanges, ChunkData, ChunkUnload, NetMovement, VIEW_RADIUS_XZ, VIEW_RADIUS_Y,
};
use deepcore_shared::voxel::{encode_chunk, BlockPos, ChunkPos};

use crate::net::handshake::ClientSession;
use crate::sim::components::{BlockChangeQueue, ServerPlayer};
use crate::world::World;

/// The chunks a client currently has loaded.
#[derive(Component, Default)]
pub struct ClientInterest {
    /// Chunks already sent.
    pub chunks: Vec<ChunkPos>,
    /// Chunk the player was in last tick.
    pub center: Option<ChunkPos>,
}

/// Chunk streaming plugin.
pub struct StreamPlugin;

impl Plugin for StreamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChunkByteCache>().add_systems(
            FixedUpdate,
            (update_interest, stream_chunks, broadcast_block_changes)
                .chain()
                .run_if(in_state(ServerState::Running)),
        );
    }
}

/// Cache of chunk bytes so a chunk is compressed once, not once per client.
#[derive(Resource, Default)]
pub struct ChunkByteCache {
    /// Encoded chunks, keyed by position.
    pub encoded: HashMap<ChunkPos, Vec<u8>>,
}

impl ChunkByteCache {
    /// Drops the cached bytes for a chunk that changed.
    pub fn invalidate(&mut self, pos: ChunkPos) {
        self.encoded.remove(&pos);
    }
}

fn player_chunk(players: &Query<(&ServerPlayer, &NetMovement)>, entity: Entity) -> Option<ChunkPos> {
    players
        .get(entity)
        .ok()
        .map(|(_, m)| ChunkPos::from_block(BlockPos::from_world(m.0.position)))
}

/// Asks the world for every chunk any player can see.
fn update_interest(
    mut world: ResMut<World>,
    sessions: Query<&ClientSession>,
    players: Query<(&ServerPlayer, &NetMovement)>,
) {
    let mut needed: Vec<ChunkPos> = Vec::new();
    for session in sessions.iter() {
        let Some(player_entity) = session.player else {
            continue;
        };
        let Some(center) = player_chunk(&players, player_entity) else {
            continue;
        };
        for pos in World::interest_set(center, VIEW_RADIUS_XZ, VIEW_RADIUS_Y) {
            if !needed.contains(&pos) {
                needed.push(pos);
            }
        }
    }
    for pos in needed {
        world.request_chunk(pos);
    }
}

/// Sends newly generated chunks to clients and unloads chunks they left.
fn stream_chunks(
    mut world: ResMut<World>,
    mut cache: ResMut<ChunkByteCache>,
    mut clients: Query<(Entity, &mut ClientInterest, &ClientSession)>,
    players: Query<(&ServerPlayer, &NetMovement)>,
    mut chunk_messages: MessageWriter<ToClients<ChunkData>>,
    mut unload_messages: MessageWriter<ToClients<ChunkUnload>>,
) {
    // Apply finished generation first so we never stream a half-built chunk.
    let generated = world.poll_chunks(256);
    if !generated.is_empty() {
        tracing::debug!("generated {} chunks ({} loaded, {} pending)", generated.len(), world.chunks.len(), world.pending.len());
    }
    for pos in generated {
        cache.invalidate(pos);
    }

    for (client_entity, mut interest, session) in clients.iter_mut() {
        let Some(player_entity) = session.player else {
            continue;
        };
        let Some(center) = player_chunk(&players, player_entity) else {
            continue;
        };
        let wanted = World::interest_set(center, VIEW_RADIUS_XZ, VIEW_RADIUS_Y);
        let target = SendTargets::Single(ClientId::Client(client_entity));

        // Unload chunks outside the radius.
        let stale: Vec<ChunkPos> = interest
            .chunks
            .iter()
            .copied()
            .filter(|c| !wanted.contains(c))
            .collect();
        if !stale.is_empty() {
            unload_messages.write(ToClients {
                targets: target,
                message: ChunkUnload { positions: stale.clone() },
            });
            interest.chunks.retain(|c| !stale.contains(c));
        }

        // Send up to a few new chunks per tick, nearest first.
        // Send more per tick while the client is still filling its view, then throttle.
        let backlog = wanted.len().saturating_sub(interest.chunks.len());
        let budget = if backlog > 64 { 24 } else { 4 };
        let mut sent = 0;
        for pos in wanted {
            if sent >= budget {
                break;
            }
            if interest.chunks.contains(&pos) || !world.is_loaded(pos) {
                continue;
            }
            let bytes = match cache.encoded.get(&pos) {
                Some(bytes) => bytes.clone(),
                None => {
                    let Some(chunk) = world.chunks.get(pos) else {
                        continue;
                    };
                    let bytes = encode_chunk(chunk);
                    cache.encoded.insert(pos, bytes.clone());
                    bytes
                }
            };
            chunk_messages.write(ToClients {
                targets: target,
                message: ChunkData { pos, bytes },
            });
            interest.chunks.push(pos);
            sent += 1;
        }
        interest.center = Some(center);
    }
}

/// Batches block changes into one message per tick, sent to every client.
fn broadcast_block_changes(
    mut queue: ResMut<BlockChangeQueue>,
    mut messages: MessageWriter<ToClients<BlockChanges>>,
) {
    if queue.changes.is_empty() {
        return;
    }
    let changes = std::mem::take(&mut queue.changes);
    messages.write(ToClients {
        targets: SendTargets::All,
        message: BlockChanges { changes },
    });
}
