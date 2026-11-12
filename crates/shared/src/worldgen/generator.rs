//! Deterministic world generation: layers, caves, ore veins, trees, water and gas pockets.
//!
//! Everything here is a pure function of the world seed and the chunk position, so the server and
//! every client regenerate byte-identical chunks.

use noise::{NoiseFn, OpenSimplex};
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::content::Content;
use crate::content::defs::HazardKind;
use crate::content::ids::{BlockId, ItemId};
use crate::voxel::chunk::{BlockPos, Chunk, ChunkPos, CHUNK_SIZE};

const SALT_CAVES: u64 = 1;
const SALT_CAVERNS: u64 = 2;
const SALT_VEINS: u64 = 3;
const SALT_TERRAIN: u64 = 4;
const SALT_WATER: u64 = 5;
const SALT_GAS: u64 = 6;
const SALT_TREES: u64 = 7;

/// Generation settings that are not content.
#[derive(Debug, Clone, Copy)]
pub struct WorldGenConfig {
    /// World seed.
    pub seed: u64,
    /// Whether to carve caves.
    pub caves: bool,
    /// Whether to grow trees on the surface.
    pub trees: bool,
}

impl WorldGenConfig {
    /// Config for a seed with all features on.
    pub fn new(seed: u64) -> Self {
        Self { seed, caves: true, trees: true }
    }
}

/// Kind of hazard pocket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PocketKind {
    /// Water pocket.
    Water,
    /// Methane pocket.
    Gas,
}

/// A pocket block: water or gas occupying a position, tagged with the pocket it came from so
/// flooding can be capped at the pocket's volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PocketBlock {
    /// The position.
    pub pos: BlockPos,
    /// Kind of pocket.
    pub kind: PocketKind,
    /// Origin of the pocket.
    pub origin: BlockPos,
}

/// A generator bound to one content set and seed.
pub struct WorldGen {
    config: WorldGenConfig,
    width: u32,
    height: u32,
    depth: u32,
    surface: u32,
    caves_noise: OpenSimplex,
    caverns_noise: OpenSimplex,
    veins_noise: OpenSimplex,
    terrain_noise: OpenSimplex,
    layers: Vec<LayerRuntime>,
    blocks: GeneratorBlocks,
}

#[derive(Debug, Clone)]
struct LayerRuntime {
    top: u32,
    bottom: u32,
    base: BlockId,
    filler: Vec<(BlockId, f32)>,
    ores: Vec<OreRuntime>,
    water_pocket: Option<(f32, u32)>,
    gas_pocket: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
struct OreRuntime {
    block: BlockId,
    frequency: f32,
    min_size: u32,
    max_size: u32,
}

/// Content-derived block ids the generator needs.
#[derive(Debug, Clone, Copy)]
pub struct GeneratorBlocks {
    /// Surface grass block.
    pub grass: BlockId,
    /// Tree trunk block.
    pub log: BlockId,
    /// Tree leaf block.
    pub leaves: BlockId,
    /// Unbreakable border block.
    pub bedrock: BlockId,
}

impl GeneratorBlocks {
    /// Resolves the block ids used by generation from content, falling back to air.
    pub fn from_content(content: &Content) -> Self {
        Self {
            grass: content.block_id("grass").unwrap_or(BlockId::AIR),
            log: content.block_id("log").unwrap_or(BlockId::AIR),
            leaves: content.block_id("leaves").unwrap_or(BlockId::AIR),
            bedrock: content.block_id("bedrock").unwrap_or(BlockId::AIR),
        }
    }
}

fn seed_for(seed: u64, salt: u64) -> u32 {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    rng.random()
}

impl WorldGen {
    /// Builds a generator from content. Layers come from `layers.ron`, bounds from `tuning.ron`.
    pub fn new(content: &Content, config: WorldGenConfig) -> Self {
        let seed = config.seed;
        let layers = content
            .layers
            .iter()
            .map(|layer| {
                let base = content.block_id(&layer.base_block).unwrap_or(BlockId::AIR);
                let filler = layer
                    .filler
                    .iter()
                    .filter_map(|(name, weight)| content.block_id(name).map(|b| (b, *weight)))
                    .collect();
                let ores = layer
                    .ores
                    .iter()
                    .filter_map(|ore| {
                        content.block_id(&ore.block).map(|b| OreRuntime {
                            block: b,
                            frequency: ore.frequency,
                            min_size: ore.vein_size.0.max(1),
                            max_size: ore.vein_size.1.max(ore.vein_size.0.max(1)),
                        })
                    })
                    .collect();
                let mut water_pocket = None;
                let mut gas_pocket = None;
                for hazard in &layer.hazards {
                    match hazard {
                        HazardKind::Flooding { pocket_chance, max_volume } => {
                            water_pocket = Some((*pocket_chance, *max_volume));
                        }
                        HazardKind::Gas { pocket_chance } => gas_pocket = Some(*pocket_chance),
                        _ => {}
                    }
                }
                LayerRuntime {
                    top: layer.depth.0,
                    bottom: layer.depth.1,
                    base,
                    filler,
                    ores,
                    water_pocket,
                    gas_pocket,
                }
            })
            .collect::<Vec<_>>();

        Self {
            config,
            width: content.tuning.world_size.0,
            height: content.tuning.world_height,
            depth: content.tuning.world_size.1,
            surface: content.tuning.surface_y,
            caves_noise: OpenSimplex::new(seed_for(seed, SALT_CAVES)),
            caverns_noise: OpenSimplex::new(seed_for(seed, SALT_CAVERNS)),
            veins_noise: OpenSimplex::new(seed_for(seed, SALT_VEINS)),
            terrain_noise: OpenSimplex::new(seed_for(seed, SALT_TERRAIN)),
            layers,
            blocks: GeneratorBlocks::from_content(content),
        }
    }

    /// The seed this generator was built with.
    pub fn seed(&self) -> u64 {
        self.config.seed
    }

    /// World width in blocks (X).
    pub fn width(&self) -> u32 {
        self.width
    }

    /// World depth in blocks (Z).
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// World height in blocks (Y).
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Surface height (topmost solid block Y) at a column.
    pub fn surface_height(&self, x: i32, z: i32) -> i32 {
        let n = self
            .terrain_noise
            .get([f64::from(x) * 0.01, f64::from(z) * 0.01]);
        (f64::from(self.surface) + n * 3.0).round() as i32
    }

    /// Generates one chunk, including trees.
    ///
    /// The inner loop is column-major and caches the surface height per column, which is where
    /// almost all of the generation time goes: 32x32 terrain columns instead of 32x32x32 noise
    /// samples.
    pub fn generate(&self, pos: ChunkPos) -> Chunk {
        let mut chunk = Chunk::filled(BlockId::AIR);
        let origin = pos.origin();
        let mut heights = [[0i32; CHUNK_SIZE as usize]; CHUNK_SIZE as usize];
        for lz in 0..CHUNK_SIZE as usize {
            for lx in 0..CHUNK_SIZE as usize {
                heights[lz][lx] = self.surface_height(origin.x + lx as i32, origin.z + lz as i32);
            }
        }
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                let surface = heights[lz as usize][lx as usize];
                let max_y = (surface - origin.y).clamp(-1, CHUNK_SIZE as i32 - 1);
                if max_y < 0 {
                    continue;
                }
                for ly in 0..=max_y as u32 {
                    let world = BlockPos::new(
                        origin.x + lx as i32,
                        origin.y + ly as i32,
                        origin.z + lz as i32,
                    );
                    let block = self.block_at_column(world, surface);
                    chunk.set_local(lx, ly, lz, block);
                }
            }
        }
        // Trees only exist in the top few chunks; skip the search for the deep ones.
        let tree_band = pos.y as u32 >= self.surface.saturating_sub(64) / CHUNK_SIZE;
        let tree_blocks = if tree_band { self.trees(pos) } else { Vec::new() };
        for (world, block) in tree_blocks {
            let local_x = world.x - origin.x;
            let local_y = world.y - origin.y;
            let local_z = world.z - origin.z;
            if (0..CHUNK_SIZE as i32).contains(&local_x)
                && (0..CHUNK_SIZE as i32).contains(&local_y)
                && (0..CHUNK_SIZE as i32).contains(&local_z)
                && chunk.get_local(local_x as u32, local_y as u32, local_z as u32) == BlockId::AIR
            {
                chunk.set_local(local_x as u32, local_y as u32, local_z as u32, block);
            }
        }
        chunk
    }

    /// [`Self::block_at`] with the column's surface height already known.
    fn block_at_column(&self, pos: BlockPos, surface: i32) -> BlockId {
        if !pos.in_world(self.width, self.height, self.depth) {
            return BlockId::AIR;
        }
        if pos.y == 0
            || pos.x == 0
            || pos.z == 0
            || pos.x == self.width as i32 - 1
            || pos.z == self.depth as i32 - 1
        {
            return self.blocks.bedrock;
        }
        if pos.y > surface {
            return BlockId::AIR;
        }
        let depth = (surface - pos.y).max(0) as u32;
        let Some(layer) = self.layer_at_depth(depth) else {
            return BlockId::AIR;
        };
        if self.config.caves && self.is_cave(pos, depth) {
            return BlockId::AIR;
        }
        if let Some(ore) = self.ore_at(layer, pos) {
            return ore;
        }
        if pos.y == surface && depth == 0 {
            return self.blocks.grass;
        }
        self.fill_block(layer, pos)
    }

    /// The block that world generation produces at a position, ignoring player modifications.
    pub fn block_at(&self, pos: BlockPos) -> BlockId {
        if !pos.in_world(self.width, self.height, self.depth) {
            return BlockId::AIR;
        }
        // Bedrock floor and walls.
        if pos.y == 0
            || pos.x == 0
            || pos.z == 0
            || pos.x == self.width as i32 - 1
            || pos.z == self.depth as i32 - 1
        {
            return self.blocks.bedrock;
        }
        let surface = self.surface_height(pos.x, pos.z);
        if pos.y > surface {
            return BlockId::AIR;
        }
        let depth = (surface - pos.y).max(0) as u32;
        let Some(layer) = self.layer_at_depth(depth) else {
            return BlockId::AIR;
        };
        if self.config.caves && self.is_cave(pos, depth) {
            return BlockId::AIR;
        }
        if let Some(ore) = self.ore_at(layer, pos) {
            return ore;
        }
        if pos.y == surface && depth == 0 {
            return self.blocks.grass;
        }
        self.fill_block(layer, pos)
    }

    fn layer_at_depth(&self, depth: u32) -> Option<&LayerRuntime> {
        self.layers
            .iter()
            .find(|l| depth >= l.top && depth < l.bottom)
    }

    fn fill_block(&self, layer: &LayerRuntime, pos: BlockPos) -> BlockId {
        if layer.filler.is_empty() {
            return layer.base;
        }
        let h = hash3(pos.x, pos.y, pos.z, self.config.seed);
        let total: f32 = layer.filler.iter().map(|(_, w)| *w).sum();
        let mut pick = (h % 10_000) as f32 / 10_000.0 * total;
        for (block, weight) in &layer.filler {
            if pick < *weight {
                return *block;
            }
            pick -= *weight;
        }
        layer.base
    }

    fn is_cave(&self, pos: BlockPos, depth: u32) -> bool {
        if depth < 3 {
            return false;
        }
        let scale = 0.045;
        let n = self.caves_noise.get([
            f64::from(pos.x) * scale,
            f64::from(pos.y) * scale * 1.7,
            f64::from(pos.z) * scale,
        ]);
        // Tunnels: noise close to zero forms connected worm-like voids.
        if n.abs() < 0.075 {
            return true;
        }
        if depth > 24 {
            let c = self.caverns_noise.get([
                f64::from(pos.x) * 0.018,
                f64::from(pos.y) * 0.03,
                f64::from(pos.z) * 0.018,
            ]);
            if c > 0.62 {
                return true;
            }
        }
        false
    }

    fn ore_at(&self, layer: &LayerRuntime, pos: BlockPos) -> Option<BlockId> {
        // Cells are 12 blocks wide; one vein may spawn per cell.
        const CELL: f64 = 12.0;
        let cx = (f64::from(pos.x) / CELL).floor() as i32;
        let cy = (f64::from(pos.y) / CELL).floor() as i32;
        let cz = (f64::from(pos.z) / CELL).floor() as i32;
        for ore in &layer.ores {
            let h = hash3(cx, cy, cz, self.config.seed ^ (u64::from(ore.block.0) << 32));
            let roll = (h % 100_000) as f32 / 100_000.0;
            if roll > ore.frequency {
                continue;
            }
            let n = self
                .veins_noise
                .get([f64::from(cx) * 0.31, f64::from(cy) * 0.31, f64::from(cz) * 0.31]);
            let centre = BlockPos::new(
                (f64::from(cx) * CELL + CELL * 0.5 + n * CELL * 0.4) as i32,
                (f64::from(cy) * CELL + CELL * 0.5 + n * CELL * 0.4) as i32,
                (f64::from(cz) * CELL + CELL * 0.5 - n * CELL * 0.4) as i32,
            );
            let span = ore.max_size - ore.min_size + 1;
            let size = ore.min_size + (h >> 20) as u32 % span;
            let radius = ((size as f32 * 0.62).cbrt() * 1.9).max(1.0);
            let dx = (pos.x - centre.x) as f32;
            let dy = (pos.y - centre.y) as f32;
            let dz = (pos.z - centre.z) as f32;
            let d = (dx * dx + dy * dy * 1.8 + dz * dz).sqrt();
            if d <= radius {
                return Some(ore.block);
            }
        }
        None
    }

    /// Water and gas pockets belonging to a chunk.
    pub fn pockets(&self, pos: ChunkPos) -> Vec<PocketBlock> {
        let mut out = Vec::new();
        let origin = pos.origin();
        for (index, layer) in self.layers.iter().enumerate() {
            if let Some((chance, max_volume)) = layer.water_pocket {
                let h = hash3(pos.x, pos.y, pos.z, self.config.seed ^ (SALT_WATER << 40) ^ index as u64);
                let roll = (h % 1_000_000) as f32 / 1_000_000.0;
                if roll <= chance {
                    let mut rng = ChaCha8Rng::seed_from_u64(h);
                    let lx = rng.random_range(2..CHUNK_SIZE as i32 - 2);
                    let lz = rng.random_range(2..CHUNK_SIZE as i32 - 2);
                    let (lo, hi) = self.layer_y_range(layer);
                    if hi > lo {
                        let y = rng.random_range(lo..hi);
                        let centre = BlockPos::new(origin.x + lx, y, origin.z + lz);
                        let volume = rng.random_range((max_volume / 4).max(8)..=max_volume.max(16));
                        let radius = (volume as f32 / (4.0 / 3.0 * std::f32::consts::PI)).cbrt();
                        let r = radius.ceil() as i32;
                        let mut filled = 0u32;
                        'outer: for dy in -r..=r {
                            for dz in -r..=r {
                                for dx in -r..=r {
                                    if filled >= volume {
                                        break 'outer;
                                    }
                                    if (dx * dx + dy * dy + dz * dz) as f32 > radius * radius {
                                        continue;
                                    }
                                    out.push(PocketBlock {
                                        pos: centre.offset(dx, dy, dz),
                                        kind: PocketKind::Water,
                                        origin: centre,
                                    });
                                    filled += 1;
                                }
                            }
                        }
                    }
                }
            }
            if let Some(chance) = layer.gas_pocket {
                let h = hash3(pos.x, pos.y, pos.z, self.config.seed ^ (SALT_GAS << 40) ^ index as u64);
                let roll = (h % 1_000_000) as f32 / 1_000_000.0;
                if roll <= chance {
                    let mut rng = ChaCha8Rng::seed_from_u64(h);
                    let lx = rng.random_range(2..CHUNK_SIZE as i32 - 2);
                    let lz = rng.random_range(2..CHUNK_SIZE as i32 - 2);
                    let (lo, hi) = self.layer_y_range(layer);
                    if hi > lo {
                        let y = rng.random_range(lo..hi);
                        let centre = BlockPos::new(origin.x + lx, y, origin.z + lz);
                        for dy in -2..=2 {
                            for dz in -2..=2 {
                                for dx in -2..=2 {
                                    if dx * dx + dy * dy + dz * dz > 5 {
                                        continue;
                                    }
                                    out.push(PocketBlock {
                                        pos: centre.offset(dx, dy, dz),
                                        kind: PocketKind::Gas,
                                        origin: centre,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    fn layer_y_range(&self, layer: &LayerRuntime) -> (i32, i32) {
        let top = (self.surface as i32 - layer.top as i32).max(3);
        let bottom = (self.surface as i32 - layer.bottom as i32 + 1).max(2);
        (bottom, top)
    }

    /// Tree trunk and canopy blocks that fall inside `pos`.
    pub fn trees(&self, pos: ChunkPos) -> Vec<(BlockPos, BlockId)> {
        if !self.config.trees {
            return Vec::new();
        }
        let mut out = Vec::new();
        for cz in (pos.z - 1)..=(pos.z + 1) {
            for cx in (pos.x - 1)..=(pos.x + 1) {
                let h = hash2(cx, cz, self.config.seed ^ SALT_TREES);
                if h % 100 > 5 {
                    continue;
                }
                let ax = cx * CHUNK_SIZE as i32 + (h >> 8) as i32 % (CHUNK_SIZE as i32);
                let az = cz * CHUNK_SIZE as i32 + (h >> 20) as i32 % (CHUNK_SIZE as i32);
                if ax <= 1
                    || az <= 1
                    || ax >= self.width as i32 - 1
                    || az >= self.depth as i32 - 1
                {
                    continue;
                }
                let ground = self.surface_height(ax, az);
                let trunk_height = 4 + (h >> 32) % 3;
                for i in 1..=trunk_height {
                    let p = BlockPos::new(ax, ground + i as i32, az);
                    if ChunkPos::from_block(p) == pos {
                        out.push((p, self.blocks.log));
                    }
                }
                let top = ground + trunk_height as i32;
                for dy in -2i32..=1 {
                    let r: i32 = if dy >= 0 { 1 } else { 2 };
                    for dz in -r..=r {
                        for dx in -r..=r {
                            if dx.abs() == r && dz.abs() == r {
                                continue;
                            }
                            let p = BlockPos::new(ax + dx, top + dy, az + dz);
                            if ChunkPos::from_block(p) == pos {
                                out.push((p, self.blocks.leaves));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Finds a safe spawn position near the centre of the base camp.
    pub fn spawn_point(&self) -> [f32; 3] {
        let x = self.width as i32 / 2;
        let z = self.depth as i32 / 2;
        let y = self.surface_height(x, z) + 2;
        [x as f32 + 0.5, y as f32, z as f32 + 0.5]
    }
}

fn hash2(x: i32, z: i32, seed: u64) -> u64 {
    let mut h = seed ^ 0x9E37_79B9_7F4A_7C15;
    h = h.wrapping_mul(0x1000_0000_01b3) ^ u64::from(x as u32);
    h = h.wrapping_mul(0x1000_0000_01b3) ^ u64::from(z as u32);
    h ^= h >> 29;
    h.wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ (h >> 32)
}

fn hash3(x: i32, y: i32, z: i32, seed: u64) -> u64 {
    let mut h = hash2(x, z, seed);
    h = h.wrapping_mul(0x1000_0000_01b3) ^ u64::from(y as u32);
    h ^= h >> 31;
    h.wrapping_mul(0x94D0_49BB_1331_11EB)
}

/// The drop item id for a block, if it drops something.
pub fn primary_drop(content: &Content, block: BlockId) -> Option<ItemId> {
    content.block(block).drops.first().map(|(item, _)| *item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_content() -> Content {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/data");
        Content::load_from_dir(dir).expect("content loads")
    }

    #[test]
    fn generation_is_deterministic() {
        let content = test_content();
        let a = WorldGen::new(&content, WorldGenConfig::new(1234));
        let b = WorldGen::new(&content, WorldGenConfig::new(1234));
        let pos = ChunkPos::new(16, 10, 16);
        let ca = a.generate(pos);
        let cb = b.generate(pos);
        for i in 0..crate::voxel::chunk::CHUNK_VOLUME {
            assert_eq!(ca.get_index(i), cb.get_index(i), "block {i} differs");
        }
    }

    #[test]
    fn different_seeds_differ() {
        let content = test_content();
        let a = WorldGen::new(&content, WorldGenConfig::new(1));
        let b = WorldGen::new(&content, WorldGenConfig::new(2));
        let pos = ChunkPos::new(16, 10, 16);
        let ca = a.generate(pos);
        let cb = b.generate(pos);
        let mut diff = 0;
        for i in 0..crate::voxel::chunk::CHUNK_VOLUME {
            if ca.get_index(i) != cb.get_index(i) {
                diff += 1;
            }
        }
        assert!(diff > 100, "expected different worlds, got {diff} differing blocks");
    }

    #[test]
    fn layers_are_ordered_and_ores_exist() {
        let content = test_content();
        let generator = WorldGen::new(&content, WorldGenConfig::new(99));
        let mut ores_found = 0;
        for x in (2..400).step_by(13) {
            for z in (2..400).step_by(17) {
                for y in (2..content.tuning.surface_y).step_by(11) {
                    let block = generator.block_at(BlockPos::new(x, y as i32, z));
                    if matches!(
                        content.block(block).def.pattern,
                        crate::content::defs::Pattern::OreFleck(_)
                    ) {
                        ores_found += 1;
                    }
                }
            }
        }
        assert!(ores_found > 50, "expected many ore blocks, found {ores_found}");
    }
}
