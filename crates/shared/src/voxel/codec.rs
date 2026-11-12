//! Chunk serialization: palette + zstd for the network and for region files.

use serde::{Deserialize, Serialize};

use crate::content::ids::BlockId;
use crate::voxel::chunk::{Chunk, CHUNK_VOLUME};

/// The wire/file representation of a chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodedChunk {
    /// Palette of block ids.
    pub palette: Vec<u16>,
    /// Packed 4-bit indices, if palette storage is used.
    pub packed: Option<Vec<u8>>,
    /// Raw ids, if raw storage is used.
    pub raw: Option<Vec<u16>>,
}

/// Compresses a chunk into bytes.
pub fn encode_chunk(chunk: &Chunk) -> Vec<u8> {
    let encoded = EncodedChunk {
        palette: chunk.palette().iter().map(|b| b.0).collect(),
        packed: chunk.packed_indices().map(|p| p.to_vec()),
        raw: chunk.raw_blocks().map(|r| r.to_vec()),
    };
    let bytes = postcard::to_allocvec(&encoded).unwrap_or_default();
    zstd::bulk::compress(&bytes, 3).unwrap_or(bytes)
}

/// Decompresses bytes produced by [`encode_chunk`].
pub fn decode_chunk(bytes: &[u8]) -> Option<Chunk> {
    let decompressed = zstd::bulk::decompress(bytes, 1 << 22).ok()?;
    let encoded: EncodedChunk = postcard::from_bytes(&decompressed).ok()?;
    let palette: Vec<BlockId> = encoded.palette.into_iter().map(BlockId).collect();
    if encoded.packed.is_some() && encoded.packed.as_ref().is_some_and(|p| p.len() != CHUNK_VOLUME / 2) {
        return None;
    }
    if encoded.raw.as_ref().is_some_and(|r| r.len() != CHUNK_VOLUME) {
        return None;
    }
    Some(Chunk::from_parts(palette, encoded.packed, encoded.raw))
}

/// An uncompressed, human-debuggable view of a chunk, used by tests and the balance tool.
pub fn chunk_summary(chunk: &Chunk) -> String {
    format!(
        "palette={} raw={} non_air={}",
        chunk.palette().len(),
        chunk.is_raw(),
        chunk.non_air_count()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_uniform() {
        let chunk = Chunk::filled(BlockId(7));
        let bytes = encode_chunk(&chunk);
        let back = decode_chunk(&bytes).expect("decodes");
        assert_eq!(back.get_index(0), BlockId(7));
        assert_eq!(back.non_air_count(), CHUNK_VOLUME as u32);
    }

    #[test]
    fn round_trip_mixed_and_raw() {
        let mut chunk = Chunk::filled(BlockId::AIR);
        for i in 0..CHUNK_VOLUME {
            chunk.set_index(i, BlockId(((i % 40) + 1) as u16));
        }
        assert!(chunk.is_raw());
        let bytes = encode_chunk(&chunk);
        let back = decode_chunk(&bytes).expect("decodes");
        for i in 0..CHUNK_VOLUME {
            assert_eq!(back.get_index(i), chunk.get_index(i));
        }
    }

    #[test]
    fn round_trip_small_palette() {
        let mut chunk = Chunk::filled(BlockId(1));
        for i in (0..CHUNK_VOLUME).step_by(3) {
            chunk.set_index(i, BlockId(2));
        }
        assert!(!chunk.is_raw());
        let bytes = encode_chunk(&chunk);
        let back = decode_chunk(&bytes).expect("decodes");
        for i in (0..CHUNK_VOLUME).step_by(3) {
            assert_eq!(back.get_index(i), BlockId(2));
        }
    }
}
