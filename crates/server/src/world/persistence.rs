//! Persistence: region files for modified chunks, SQLite-free JSON records for players and base.
//!
//! Writes are crash-safe: data goes to a temporary file which is then renamed over the target.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use deepcore_shared::content::ids::{BlockId, ItemId, MachineId, RecipeId, TechId};
use deepcore_shared::inventory::Inventory;
use deepcore_shared::voxel::{BlockPos, ChunkPos};
use deepcore_shared::Vitals;

/// A saved player record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerRecord {
    /// Account or display name.
    pub name: String,
    /// Position.
    pub position: [f32; 3],
    /// Inventory.
    pub inventory: Inventory,
    /// Vitals.
    pub vitals: Vitals,
}

/// A saved machine record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineRecord {
    /// Machine type.
    pub machine: MachineId,
    /// Position.
    pub pos: BlockPos,
    /// Facing.
    pub facing: u8,
    /// Selected recipe.
    pub recipe: Option<RecipeId>,
    /// Input stacks as `(slot, item, count)`.
    pub inputs: Vec<(u8, ItemId, u16)>,
    /// Output stacks.
    pub outputs: Vec<(u8, ItemId, u16)>,
    /// Fuel stacks.
    pub fuel: Vec<(u8, ItemId, u16)>,
    /// Remaining burn time.
    pub burn_remaining: f32,
    /// Reactor core temperature.
    pub core_temp: f32,
    /// Reactor rod insertion.
    pub rod_insertion: f32,
    /// Whether the reactor is scrammed.
    pub scrameed: bool,
}

/// The complete saved state of a world.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SaveGame {
    /// World seed.
    pub seed: u64,
    /// Content hash the save was made with.
    pub content_hash: u64,
    /// Modified blocks.
    pub edits: Vec<(BlockPos, BlockId)>,
    /// Placed machines.
    pub machines: Vec<MachineRecord>,
    /// Player records.
    pub players: Vec<PlayerRecord>,
    /// Researched tech nodes.
    pub researched: Vec<TechId>,
    /// Research progress.
    pub research_progress: Vec<(TechId, Vec<(ItemId, u16)>)>,
}

/// One region file: an 8×8×8 block of chunks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegionFile {
    /// Chunk index inside the region → compressed chunk bytes.
    pub chunks: HashMap<(u8, u8, u8), Vec<u8>>,
}

/// A save directory with region files and the world record.
pub struct SaveStore {
    root: PathBuf,
}

fn region_of(pos: ChunkPos) -> (i32, i32, i32) {
    (pos.x.div_euclid(8), pos.y.div_euclid(8), pos.z.div_euclid(8))
}

fn region_key(pos: ChunkPos) -> (u8, u8, u8) {
    (
        pos.x.rem_euclid(8) as u8,
        pos.y.rem_euclid(8) as u8,
        pos.z.rem_euclid(8) as u8,
    )
}

impl SaveStore {
    /// Opens (and creates) a save directory.
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("regions"))?;
        Ok(Self { root })
    }

    fn region_path(&self, region: (i32, i32, i32)) -> PathBuf {
        self.root
            .join("regions")
            .join(format!("r.{}.{}.{}.bin", region.0, region.1, region.2))
    }

    /// Writes chunk bytes into their region file, merging with existing contents.
    pub fn write_chunk(&self, pos: ChunkPos, bytes: Vec<u8>) -> std::io::Result<()> {
        let region = region_of(pos);
        let path = self.region_path(region);
        let mut file: RegionFile = if path.exists() {
            let data = std::fs::read(&path)?;
            postcard::from_bytes(&data).unwrap_or_default()
        } else {
            RegionFile::default()
        };
        file.chunks.insert(region_key(pos), bytes);
        let encoded = postcard::to_allocvec(&file)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        atomic_write(&path, &encoded)
    }

    /// Reads chunk bytes from their region file.
    pub fn read_chunk(&self, pos: ChunkPos) -> Option<Vec<u8>> {
        let path = self.region_path(region_of(pos));
        let data = std::fs::read(path).ok()?;
        let file: RegionFile = postcard::from_bytes(&data).ok()?;
        file.chunks.get(&region_key(pos)).cloned()
    }

    /// Writes the world record.
    pub fn write_save(&self, save: &SaveGame) -> std::io::Result<()> {
        let encoded = ron::ser::to_string_pretty(save, ron::ser::PrettyConfig::default())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        atomic_write(&self.root.join("world.ron"), encoded.as_bytes())
    }

    /// Reads the world record.
    pub fn read_save(&self) -> Option<SaveGame> {
        let text = std::fs::read_to_string(self.root.join("world.ron")).ok()?;
        ron::from_str(&text).ok()
    }

    /// The directory backing this store.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Writes a file by writing a temp file first and renaming it into place.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_chunks() {
        let dir = std::env::temp_dir().join(format!("deepcore-save-test-{}", std::process::id()));
        let store = SaveStore::new(&dir).expect("store");
        let pos = ChunkPos::new(3, 4, 5);
        store.write_chunk(pos, vec![1, 2, 3]).expect("write");
        assert_eq!(store.read_chunk(pos), Some(vec![1, 2, 3]));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trip_save() {
        let dir = std::env::temp_dir().join(format!("deepcore-save-test2-{}", std::process::id()));
        let store = SaveStore::new(&dir).expect("store");
        let mut save = SaveGame::default();
        save.seed = 42;
        save.edits.push((BlockPos::new(1, 2, 3), BlockId(7)));
        store.write_save(&save).expect("write");
        let back = store.read_save().expect("read");
        assert_eq!(back.seed, 42);
        assert_eq!(back.edits.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
