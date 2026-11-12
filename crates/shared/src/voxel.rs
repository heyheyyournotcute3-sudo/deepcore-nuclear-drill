//! Voxel storage: chunks, positions and ray casting.

pub mod chunk;
pub mod codec;
pub mod raycast;

pub use crate::content::ids::BlockId;
pub use chunk::{
    BlockChange, BlockPos, Chunk, ChunkMap, ChunkPos, Face, CHUNKS_PER_AXIS, CHUNKS_Y, CHUNK_SIZE,
    CHUNK_VOLUME,
};
pub use codec::{decode_chunk, encode_chunk};
pub use raycast::{face_area, line_of_sight, raycast, scan_radius, ChunkLookup, RayHit};
