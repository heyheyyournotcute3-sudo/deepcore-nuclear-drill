//! Chunk storage with palette compression, plus position maths.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::content::ids::BlockId;

/// Edge length of a chunk in blocks.
pub const CHUNK_SIZE: u32 = 32;
/// Number of blocks in a chunk.
pub const CHUNK_VOLUME: usize = (CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE) as usize;
/// Number of chunks along the world's X and Z axes (1024 / 32).
pub const CHUNKS_PER_AXIS: u32 = 32;
/// Number of chunks along the world's Y axis (512 / 32).
pub const CHUNKS_Y: u32 = 16;

const CHUNK_SIZE_I: i32 = CHUNK_SIZE as i32;
const CHUNK_SIZE_U: usize = CHUNK_SIZE as usize;

/// Index of a chunk in the chunk grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChunkPos {
    /// Chunk X.
    pub x: i32,
    /// Chunk Y (vertical).
    pub y: i32,
    /// Chunk Z.
    pub z: i32,
}

impl ChunkPos {
    /// Creates a chunk position.
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The chunk containing a block position.
    pub fn from_block(pos: BlockPos) -> Self {
        Self {
            x: pos.x.div_euclid(CHUNK_SIZE_I),
            y: pos.y.div_euclid(CHUNK_SIZE_I),
            z: pos.z.div_euclid(CHUNK_SIZE_I),
        }
    }

    /// World position of the chunk's minimum corner.
    pub fn origin(self) -> BlockPos {
        BlockPos::new(self.x * CHUNK_SIZE_I, self.y * CHUNK_SIZE_I, self.z * CHUNK_SIZE_I)
    }

    /// Linear index in the chunk grid, or `None` if outside the world.
    pub fn grid_index(self) -> Option<usize> {
        if self.x < 0
            || self.z < 0
            || self.y < 0
            || self.x >= CHUNKS_PER_AXIS as i32
            || self.z >= CHUNKS_PER_AXIS as i32
            || self.y >= CHUNKS_Y as i32
        {
            return None;
        }
        Some(
            self.x as usize
                + self.z as usize * CHUNKS_PER_AXIS as usize
                + self.y as usize * (CHUNKS_PER_AXIS * CHUNKS_PER_AXIS) as usize,
        )
    }

    /// Whether this chunk is inside the world bounds.
    pub fn in_world(self) -> bool {
        self.grid_index().is_some()
    }

    /// Chebyshev distance to another chunk in blocks of chunks.
    pub fn distance(self, other: ChunkPos) -> i32 {
        (self.x - other.x)
            .abs()
            .max((self.y - other.y).abs())
            .max((self.z - other.z).abs())
    }
}

/// A block position in world space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BlockPos {
    /// Block X.
    pub x: i32,
    /// Block Y.
    pub y: i32,
    /// Block Z.
    pub z: i32,
}

impl BlockPos {
    /// Creates a block position.
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The position offset by a delta.
    pub const fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.z + dz)
    }

    /// The position offset by a unit face normal.
    pub const fn offset_dir(self, dir: Face) -> Self {
        let (dx, dy, dz) = dir.delta();
        self.offset(dx, dy, dz)
    }

    /// Index inside the owning chunk.
    pub fn chunk_index(self) -> usize {
        let lx = self.x.rem_euclid(CHUNK_SIZE_I) as usize;
        let ly = self.y.rem_euclid(CHUNK_SIZE_I) as usize;
        let lz = self.z.rem_euclid(CHUNK_SIZE_I) as usize;
        lx + lz * CHUNK_SIZE_U + ly * CHUNK_SIZE_U * CHUNK_SIZE_U
    }

    /// Whether this position is inside the world bounds given by `width`, `height` and `depth`.
    pub fn in_world(self, width: u32, height: u32, depth: u32) -> bool {
        self.x >= 0
            && self.z >= 0
            && self.y >= 0
            && (self.x as u32) < width
            && (self.z as u32) < depth
            && (self.y as u32) < height
    }

    /// Centre of the block in world space.
    pub fn center(self) -> [f32; 3] {
        [self.x as f32 + 0.5, self.y as f32 + 0.5, self.z as f32 + 0.5]
    }

    /// Squared distance from a point to the centre of this block.
    pub fn distance_squared_to(self, p: [f32; 3]) -> f32 {
        let c = self.center();
        let dx = c[0] - p[0];
        let dy = c[1] - p[1];
        let dz = c[2] - p[2];
        dx * dx + dy * dy + dz * dz
    }

    /// Converts a float world position to the block containing it.
    pub fn from_world(p: [f32; 3]) -> Self {
        Self::new(
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        )
    }
}

/// The six axis-aligned block faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Face {
    /// Negative X.
    NegX,
    /// Positive X.
    PosX,
    /// Negative Y.
    NegY,
    /// Positive Y.
    PosY,
    /// Negative Z.
    NegZ,
    /// Positive Z.
    PosZ,
}

impl Face {
    /// All faces, in a fixed order.
    pub const ALL: [Face; 6] = [
        Face::NegX,
        Face::PosX,
        Face::NegY,
        Face::PosY,
        Face::NegZ,
        Face::PosZ,
    ];

    /// The unit delta of the face normal.
    pub const fn delta(self) -> (i32, i32, i32) {
        match self {
            Face::NegX => (-1, 0, 0),
            Face::PosX => (1, 0, 0),
            Face::NegY => (0, -1, 0),
            Face::PosY => (0, 1, 0),
            Face::NegZ => (0, 0, -1),
            Face::PosZ => (0, 0, 1),
        }
    }

    /// The opposite face.
    pub const fn opposite(self) -> Face {
        match self {
            Face::NegX => Face::PosX,
            Face::PosX => Face::NegX,
            Face::NegY => Face::PosY,
            Face::PosY => Face::NegY,
            Face::NegZ => Face::PosZ,
            Face::PosZ => Face::NegZ,
        }
    }

    /// The face whose normal points most opposite to a direction vector (i.e. the face you hit
    /// when moving along `dir`).
    pub fn facing(dir: [f32; 3]) -> Face {
        let ax = dir[0].abs();
        let ay = dir[1].abs();
        let az = dir[2].abs();
        if ax >= ay && ax >= az {
            if dir[0] > 0.0 { Face::NegX } else { Face::PosX }
        } else if ay >= az {
            if dir[1] > 0.0 { Face::NegY } else { Face::PosY }
        } else if dir[2] > 0.0 {
            Face::NegZ
        } else {
            Face::PosZ
        }
    }
}

/// A 32³ chunk of blocks, stored as a palette plus 4-bit indices when it is uniform enough,
/// and as raw `u16` ids otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Palette of block ids used by this chunk. Always non-empty.
    palette: Vec<BlockId>,
    /// Either packed 4-bit palette indices (two blocks per byte) or `None` when `raw` is used.
    packed: Option<Vec<u8>>,
    /// Raw block ids, used when the chunk has more than 16 distinct blocks.
    raw: Option<Vec<u16>>,
    /// Number of non-air blocks, for quick emptiness checks.
    non_air: u32,
}

/// A palette entry index inside a chunk.
const MAX_PALETTE: usize = 16;

impl Default for Chunk {
    fn default() -> Self {
        Self::filled(BlockId::AIR)
    }
}

impl Chunk {
    /// A chunk where every block is `block`.
    pub fn filled(block: BlockId) -> Self {
        Self {
            palette: vec![block],
            packed: None,
            raw: None,
            non_air: if block == BlockId::AIR { 0 } else { CHUNK_VOLUME as u32 },
        }
    }

    /// Whether the chunk contains no solid blocks.
    pub fn is_empty(&self) -> bool {
        self.non_air == 0
    }

    /// Number of non-air blocks.
    pub fn non_air_count(&self) -> u32 {
        self.non_air
    }

    /// The block at a local index (`0..CHUNK_VOLUME`).
    pub fn get_index(&self, index: usize) -> BlockId {
        debug_assert!(index < CHUNK_VOLUME);
        if let Some(raw) = &self.raw {
            return BlockId(raw[index]);
        }
        let pal = self.palette_index(index);
        self.palette.get(pal).copied().unwrap_or(BlockId::AIR)
    }

    fn palette_index(&self, index: usize) -> usize {
        match &self.packed {
            None => 0,
            Some(bytes) => {
                let byte = bytes[index / 2];
                let nibble = if index % 2 == 0 { byte & 0x0F } else { byte >> 4 };
                usize::from(nibble)
            }
        }
    }

    /// The block at local coordinates.
    pub fn get_local(&self, x: u32, y: u32, z: u32) -> BlockId {
        let index = x as usize + z as usize * CHUNK_SIZE_U + y as usize * CHUNK_SIZE_U * CHUNK_SIZE_U;
        self.get_index(index)
    }

    /// Sets the block at a local index, converting the storage as needed.
    pub fn set_index(&mut self, index: usize, block: BlockId) {
        debug_assert!(index < CHUNK_VOLUME);
        let previous = self.get_index(index);
        if previous == block {
            return;
        }
        if previous == BlockId::AIR && block != BlockId::AIR {
            self.non_air += 1;
        } else if previous != BlockId::AIR && block == BlockId::AIR {
            self.non_air -= 1;
        }

        if let Some(raw) = &mut self.raw {
            raw[index] = block.0;
            return;
        }

        // Try to keep the palette representation.
        if let Some(slot) = self.palette.iter().position(|b| *b == block) {
            if let Some(bytes) = &mut self.packed {
                let byte = &mut bytes[index / 2];
                if index % 2 == 0 {
                    *byte = (*byte & 0xF0) | slot as u8;
                } else {
                    *byte = (*byte & 0x0F) | ((slot as u8) << 4);
                }
            }
            return;
        }

        if self.palette.len() < MAX_PALETTE {
            self.palette.push(block);
            let slot = self.palette.len() - 1;
            let bytes = self.packed.get_or_insert_with(|| vec![0u8; CHUNK_VOLUME / 2]);
            let byte = &mut bytes[index / 2];
            if index % 2 == 0 {
                *byte = (*byte & 0xF0) | slot as u8;
            } else {
                *byte = (*byte & 0x0F) | ((slot as u8) << 4);
            }
            return;
        }

        // Palette overflow: convert to raw storage.
        let mut raw = Vec::with_capacity(CHUNK_VOLUME);
        for i in 0..CHUNK_VOLUME {
            raw.push(self.get_index(i).0);
        }
        raw[index] = block.0;
        self.raw = Some(raw);
        self.packed = None;
        self.palette = vec![BlockId::AIR];
    }

    /// Sets the block at local coordinates.
    pub fn set_local(&mut self, x: u32, y: u32, z: u32, block: BlockId) {
        let index = x as usize + z as usize * CHUNK_SIZE_U + y as usize * CHUNK_SIZE_U * CHUNK_SIZE_U;
        self.set_index(index, block);
    }

    /// The palette of this chunk (used by the network encoding).
    pub fn palette(&self) -> &[BlockId] {
        &self.palette
    }

    /// Whether this chunk uses raw storage.
    pub fn is_raw(&self) -> bool {
        self.raw.is_some()
    }

    /// Raw block ids, if the chunk uses raw storage.
    pub fn raw_blocks(&self) -> Option<&[u16]> {
        self.raw.as_deref()
    }

    /// Packed 4-bit palette indices, if the chunk uses palette storage with more than one entry.
    pub fn packed_indices(&self) -> Option<&[u8]> {
        self.packed.as_deref()
    }

    /// Rebuilds a chunk from its wire representation.
    pub fn from_parts(palette: Vec<BlockId>, packed: Option<Vec<u8>>, raw: Option<Vec<u16>>) -> Self {
        let mut chunk = Self {
            palette: if palette.is_empty() {
                vec![BlockId::AIR]
            } else {
                palette
            },
            packed,
            raw,
            non_air: 0,
        };
        let mut count = 0;
        for i in 0..CHUNK_VOLUME {
            if chunk.get_index(i) != BlockId::AIR {
                count += 1;
            }
        }
        chunk.non_air = count;
        chunk
    }

    /// Iterates over every non-air block as `(local_x, local_y, local_z, block)`.
    pub fn iter_non_air(&self) -> impl Iterator<Item = (u32, u32, u32, BlockId)> + '_ {
        (0..CHUNK_SIZE).flat_map(move |y| {
            (0..CHUNK_SIZE).flat_map(move |z| {
                (0..CHUNK_SIZE).filter_map(move |x| {
                    let block = self.get_local(x, y, z);
                    (block != BlockId::AIR).then_some((x, y, z, block))
                })
            })
        })
    }
}

/// A sparse map of loaded chunks.
#[derive(Debug, Default)]
pub struct ChunkMap {
    chunks: HashMap<ChunkPos, Chunk>,
}

impl ChunkMap {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a chunk.
    pub fn insert(&mut self, pos: ChunkPos, chunk: Chunk) -> Option<Chunk> {
        self.chunks.insert(pos, chunk)
    }

    /// Removes a chunk.
    pub fn remove(&mut self, pos: ChunkPos) -> Option<Chunk> {
        self.chunks.remove(&pos)
    }

    /// Borrows a chunk.
    pub fn get(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    /// Mutably borrows a chunk.
    pub fn get_mut(&mut self, pos: ChunkPos) -> Option<&mut Chunk> {
        self.chunks.get_mut(&pos)
    }

    /// Number of loaded chunks.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Whether no chunks are loaded.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Iterates over loaded chunk positions.
    pub fn positions(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.chunks.keys().copied()
    }

    /// Iterates over `(position, chunk)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (ChunkPos, &Chunk)> + '_ {
        self.chunks.iter().map(|(p, c)| (*p, c))
    }
}

/// A block change applied to a single position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockChange {
    /// Position that changed.
    pub pos: BlockPos,
    /// New block.
    pub block: BlockId,
}
