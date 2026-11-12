//! The authoritative world: chunk store, generation, block edits and hazard pockets.

pub mod persistence;

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;

use deepcore_shared::content::ids::BlockId;
use deepcore_shared::voxel::{BlockPos, Chunk, ChunkMap, ChunkPos, CHUNKS_Y};
use deepcore_shared::worldgen::{PocketKind, WorldGen, WorldGenConfig};
use deepcore_shared::GameContent;

/// A block edit that must be persisted.
#[derive(Debug, Clone, Copy)]
pub struct Edit {
    /// Position edited.
    pub pos: BlockPos,
    /// Block placed.
    pub block: BlockId,
}

/// The server's world state.
#[derive(Resource)]
pub struct World {
    /// The deterministic generator.
    pub generator: Arc<WorldGen>,
    /// Loaded chunks.
    pub chunks: ChunkMap,
    /// Chunks currently being generated on a worker thread.
    pub pending: HashMap<ChunkPos, bevy::tasks::Task<Chunk>>,
    /// Blocks the players have changed, which override generation.
    pub edits: HashMap<BlockPos, BlockId>,
    /// Water and gas pocket membership.
    pub water: HashMap<BlockPos, BlockPos>,
    /// Gas pocket membership.
    pub gas: HashMap<BlockPos, BlockPos>,
    /// Pocket volume accounting, keyed by pocket origin.
    pub pocket_volumes: HashMap<BlockPos, u32>,
    /// World seed.
    pub seed: u64,
    /// Accumulated set of chunks modified since the last save.
    pub dirty_chunks: Vec<ChunkPos>,
}

impl World {
    /// Creates a world for a content set and seed.
    pub fn new(content: &GameContent, seed: u64) -> Self {
        Self {
            generator: Arc::new(WorldGen::new(content, WorldGenConfig::new(seed))),
            chunks: ChunkMap::new(),
            pending: HashMap::new(),
            edits: HashMap::new(),
            water: HashMap::new(),
            gas: HashMap::new(),
            pocket_volumes: HashMap::new(),
            seed,
            dirty_chunks: Vec::new(),
        }
    }

    /// The block at a position. Unloaded chunks read as solid bedrock so players cannot fall out.
    pub fn block(&self, pos: BlockPos) -> BlockId {
        if let Some(edit) = self.edits.get(&pos) {
            return *edit;
        }
        if self.water.contains_key(&pos) {
            return BlockId::AIR;
        }
        let chunk_pos = ChunkPos::from_block(pos);
        match self.chunks.get(chunk_pos) {
            Some(chunk) => chunk.get_index(pos.chunk_index()),
            None => BlockId::AIR,
        }
    }

    /// Whether a block stops movement.
    pub fn is_solid(&self, content: &GameContent, pos: BlockPos) -> bool {
        let block = self.block(pos);
        let def = &content.block(block).def;
        def.solid && !def.liquid
    }

    /// Sets a block and records the edit.
    pub fn set_block(&mut self, pos: BlockPos, block: BlockId) {
        self.edits.insert(pos, block);
        let chunk_pos = ChunkPos::from_block(pos);
        if let Some(chunk) = self.chunks.get_mut(chunk_pos) {
            chunk.set_index(pos.chunk_index(), block);
        }
        if !self.dirty_chunks.contains(&chunk_pos) {
            self.dirty_chunks.push(chunk_pos);
        }
        self.water.remove(&pos);
        self.gas.remove(&pos);
    }

    /// Requests generation of a chunk if it is not loaded or pending.
    pub fn request_chunk(&mut self, pos: ChunkPos) {
        if !pos.in_world() || self.chunks.get(pos).is_some() || self.pending.contains_key(&pos) {
            return;
        }
        let generator = self.generator.clone();
        let pool = AsyncComputeTaskPool::get();
        let task = pool.spawn(async move { generator.generate(pos) });
        self.pending.insert(pos, task);
    }

    /// Collects finished generation tasks, applying edits and hazard pockets.
    ///
    /// Returns the positions of newly available chunks.
    pub fn poll_chunks(&mut self, limit: usize) -> Vec<ChunkPos> {
        let mut finished = Vec::new();
        let done: Vec<ChunkPos> = self
            .pending
            .keys()
            .copied()
            .filter(|pos| self.pending.get(pos).is_some_and(|t| t.is_finished()))
            .collect();
        for pos in done.into_iter().take(limit) {
            let Some(task) = self.pending.remove(&pos) else {
                continue;
            };
            let mut chunk = bevy::tasks::block_on(task);
            // Apply saved edits for this chunk.
            let origin = pos.origin();
            for y in 0..32 {
                for z in 0..32 {
                    for x in 0..32 {
                        let world_pos = origin.offset(x, y, z);
                        if let Some(block) = self.edits.get(&world_pos) {
                            chunk.set_local(x as u32, y as u32, z as u32, *block);
                        }
                    }
                }
            }
            // Hazard pockets live in caves: only fill air blocks.
            for pocket in self.generator.pockets(pos) {
                let local_x = pocket.pos.x - origin.x;
                let local_y = pocket.pos.y - origin.y;
                let local_z = pocket.pos.z - origin.z;
                if !(0..32).contains(&local_x)
                    || !(0..32).contains(&local_y)
                    || !(0..32).contains(&local_z)
                {
                    continue;
                }
                if chunk.get_local(local_x as u32, local_y as u32, local_z as u32) != BlockId::AIR {
                    continue;
                }
                let volume = self.pocket_volumes.entry(pocket.origin).or_insert(0);
                if *volume >= 512 {
                    continue;
                }
                *volume += 1;
                match pocket.kind {
                    PocketKind::Water => {
                        self.water.insert(pocket.pos, pocket.origin);
                    }
                    PocketKind::Gas => {
                        self.gas.insert(pocket.pos, pocket.origin);
                    }
                }
            }
            self.chunks.insert(pos, chunk);
            finished.push(pos);
        }
        finished
    }

    /// Whether a chunk is loaded.
    pub fn is_loaded(&self, pos: ChunkPos) -> bool {
        self.chunks.get(pos).is_some()
    }

    /// Releases chunks that no player can see.
    pub fn release_far_chunks(&mut self, keep: &[ChunkPos]) {
        let to_remove: Vec<ChunkPos> = self
            .chunks
            .positions()
            .filter(|pos| !keep.contains(pos))
            .collect();
        for pos in to_remove {
            self.chunks.remove(pos);
        }
    }

    /// Chunk positions within the interest radius of a point.
    pub fn interest_set(center: ChunkPos, radius_xz: i32, radius_y: i32) -> Vec<ChunkPos> {
        let mut out = Vec::new();
        for dy in -radius_y..=radius_y {
            for dz in -radius_xz..=radius_xz {
                for dx in -radius_xz..=radius_xz {
                    let pos = ChunkPos::new(center.x + dx, center.y + dy, center.z + dz);
                    if pos.in_world() {
                        out.push(pos);
                    }
                }
            }
        }
        // Nearest first.
        out.sort_by_key(|p| p.distance(center));
        out
    }

    /// The Y of the deepest chunk in the world.
    pub fn bottom_chunk_y() -> i32 {
        CHUNKS_Y as i32 - 1
    }
}
