//! The client's copy of the world: chunk map, block lookups and local block edits.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::protocol::{BlockChangeMsg, BlockChanges, ChunkData, ChunkUnload};
use deepcore_shared::voxel::{decode_chunk, BlockId, BlockPos, Chunk, ChunkMap, ChunkPos};
use deepcore_shared::GameContent;

/// Client settings that come from the command line and the settings menu.
#[derive(Resource, Debug, Clone)]
pub struct ClientSettings {
    /// Display name.
    pub name: String,
    /// Render distance in chunks.
    pub view_distance: i32,
    /// Simulated latency in milliseconds.
    pub simulate_latency_ms: u64,
    /// Simulated packet loss percentage.
    pub simulate_loss_percent: u8,
    /// Server to connect to on startup.
    pub initial_connect: Option<String>,
}

/// The client's chunk store.
#[derive(Resource, Default)]
pub struct ClientWorld {
    /// Loaded chunks.
    pub chunks: ChunkMap,
    /// Chunks whose mesh must be rebuilt.
    pub dirty: Vec<ChunkPos>,
}

impl ClientWorld {
    /// The block at a position; unloaded chunks read as air.
    pub fn block(&self, pos: BlockPos) -> BlockId {
        let chunk_pos = ChunkPos::from_block(pos);
        match self.chunks.get(chunk_pos) {
            Some(chunk) => chunk.get_index(pos.chunk_index()),
            None => BlockId::AIR,
        }
    }

    /// Whether a block stops movement and rays.
    pub fn is_solid(&self, content: &GameContent, pos: BlockPos) -> bool {
        let def = &content.block(self.block(pos)).def;
        def.solid && !def.liquid
    }

    /// Applies a block change from the server.
    pub fn apply_change(&mut self, change: BlockChangeMsg) {
        let chunk_pos = ChunkPos::from_block(change.pos);
        if let Some(chunk) = self.chunks.get_mut(chunk_pos) {
            chunk.set_index(change.pos.chunk_index(), change.block);
        }
        if !self.dirty.contains(&chunk_pos) {
            self.dirty.push(chunk_pos);
        }
    }

    /// Number of loaded chunks.
    pub fn loaded(&self) -> usize {
        self.chunks.len()
    }
}

/// Entities waiting to be meshed after their chunk arrives.
#[derive(Resource, Default)]
pub struct ChunkEntities {
    /// Entity per chunk.
    pub map: HashMap<ChunkPos, Entity>,
}

/// The world plugin: receives chunks and block changes.
pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientWorld>()
            .init_resource::<ChunkEntities>()
            .add_systems(
                Update,
                (receive_chunks, receive_unloads, receive_block_changes)
                    .run_if(in_state(ClientState::Connected)),
            );
    }
}

fn receive_chunks(
    mut chunks: MessageReader<ChunkData>,
    mut world: ResMut<ClientWorld>,
    mut mesher: ResMut<crate::render::Mesher>,
) {
    for message in chunks.read() {
        let Some(chunk) = decode_chunk(&message.bytes) else {
            warn!("failed to decode chunk {:?}", message.pos);
            continue;
        };
        world.chunks.insert(message.pos, chunk);
        world.dirty.push(message.pos);
        mesher.dirty.push(message.pos);
        if world.chunks.len() == 1 {
            info!("received first chunk {:?}", message.pos);
        }
    }
}

fn receive_unloads(
    mut unloads: MessageReader<ChunkUnload>,
    mut world: ResMut<ClientWorld>,
    mut entities: ResMut<ChunkEntities>,
    mut commands: Commands,
) {
    for message in unloads.read() {
        for pos in &message.positions {
            world.chunks.remove(*pos);
            if let Some(entity) = entities.map.remove(pos) {
                commands.entity(entity).despawn();
            }
        }
    }
}

fn receive_block_changes(
    mut changes: MessageReader<BlockChanges>,
    mut world: ResMut<ClientWorld>,
    mut mesher: ResMut<crate::render::Mesher>,
) {
    for message in changes.read() {
        for change in &message.changes {
            world.apply_change(*change);
            let pos = ChunkPos::from_block(change.pos);
            if !mesher.dirty.contains(&pos) {
                mesher.dirty.push(pos);
            }
        }
    }
}

/// A chunk as it is stored on the client, for tests.
pub fn chunk_or_empty(map: &ChunkMap, pos: ChunkPos) -> Chunk {
    map.get(pos).cloned().unwrap_or_default()
}
