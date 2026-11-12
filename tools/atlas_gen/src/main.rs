//! Procedural block texture atlas generator.
//!
//! Reads `blocks.ron`, paints a 16×16 tile per block from its `color` and `pattern`, and writes a
//! PNG atlas plus a JSON-free RON manifest of tile indices. No downloaded art is ever needed.

use std::path::PathBuf;

// Prints are routed through eprintln so the tool is readable in any terminal setup.

use deepcore_shared::content::Content;
use deepcore_shared::content::defs::Pattern;

/// Size of one tile in pixels.
pub const TILE: u32 = 16;
/// Tiles per row in the atlas.
pub const ATLAS_COLUMNS: u32 = 16;

/// A generated atlas: RGBA pixels plus the tile index of each block.
pub struct Atlas {
    /// Atlas width in pixels.
    pub width: u32,
    /// Atlas height in pixels.
    pub height: u32,
    /// RGBA8 pixels.
    pub pixels: Vec<u8>,
    /// Tile index per block id.
    pub tile_of_block: Vec<u32>,
}

/// Deterministic hash for texture noise.
fn hash(x: u32, y: u32, salt: u32) -> u32 {
    let mut h = x.wrapping_mul(0x1F1F_1F1F) ^ y.wrapping_mul(0x9E37_79B9) ^ salt;
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h
}

/// Builds the atlas for a content set.
pub fn build_atlas(content: &Content) -> Atlas {
    let count = content.blocks.len() as u32;
    let rows = count.div_ceil(ATLAS_COLUMNS).max(1);
    let width = ATLAS_COLUMNS * TILE;
    let height = rows * TILE;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut tile_of_block = Vec::with_capacity(content.blocks.len());

    for (index, block) in content.blocks.iter().enumerate() {
        let tile = index as u32;
        tile_of_block.push(tile);
        let tx = (tile % ATLAS_COLUMNS) * TILE;
        let ty = (tile / ATLAS_COLUMNS) * TILE;
        let def = &block.def;
        let [r, g, b] = def.color.to_array();
        for py in 0..TILE {
            for px in 0..TILE {
                let (mut cr, mut cg, mut cb) = (r, g, b);
                let n = (hash(px, py, tile) % 1000) as f32 / 1000.0;
                match def.pattern {
                    Pattern::Solid => {
                        let shade = 0.94 + n * 0.12;
                        cr *= shade;
                        cg *= shade;
                        cb *= shade;
                    }
                    Pattern::Speckle => {
                        let shade = 0.78 + n * 0.44;
                        cr *= shade;
                        cg *= shade;
                        cb *= shade;
                    }
                    Pattern::Bands => {
                        let band = if (py / 3) % 2 == 0 { 1.08 } else { 0.9 };
                        let shade = band * (0.94 + n * 0.12);
                        cr *= shade;
                        cg *= shade;
                        cb *= shade;
                    }
                    Pattern::Cracks => {
                        let crack = (px * 7 + py * 13 + hash(px / 4, py / 4, tile) % 17) % 23 == 0;
                        let shade = if crack { 0.55 } else { 0.86 + n * 0.24 };
                        cr *= shade;
                        cg *= shade;
                        cb *= shade;
                    }
                    Pattern::Glow => {
                        let dx = px as f32 - 7.5;
                        let dy = py as f32 - 7.5;
                        let d = (dx * dx + dy * dy).sqrt() / 11.0;
                        let glow = (1.0 - d).clamp(0.0, 1.0);
                        cr = (cr * 0.6 + glow * 0.8).min(1.0);
                        cg = (cg * 0.6 + glow * 0.8).min(1.0);
                        cb = (cb * 0.6 + glow * 0.8).min(1.0);
                    }
                    Pattern::OreFleck(fleck) => {
                        // Base rock with bright ore blobs.
                        let shade = 0.78 + n * 0.4;
                        cr *= shade;
                        cg *= shade;
                        cb *= shade;
                        let blob = hash(px / 3, py / 3, tile ^ 0xABCD) % 100 < 26;
                        if blob {
                            cr = fleck.0;
                            cg = fleck.1;
                            cb = fleck.2;
                        }
                    }
                }
                // Emissive blocks are brightened so they read as light sources.
                let emissive = def.emissive.clamp(0.0, 1.0);
                cr = (cr + emissive * 0.35).min(1.0);
                cg = (cg + emissive * 0.35).min(1.0);
                cb = (cb + emissive * 0.35).min(1.0);

                let offset = (((ty + py) * width + tx + px) * 4) as usize;
                pixels[offset] = linear_to_srgb(cr);
                pixels[offset + 1] = linear_to_srgb(cg);
                pixels[offset + 2] = linear_to_srgb(cb);
                pixels[offset + 3] = if block.def.liquid { 190 } else { 255 };
            }
        }
    }

    Atlas { width, height, pixels, tile_of_block }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

impl Atlas {
    /// Writes the atlas as a PNG.
    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let buffer = image::RgbaImage::from_raw(self.width, self.height, self.pixels.clone())
            .ok_or_else(|| "atlas buffer size mismatch".to_string())?;
        buffer.save(path).map_err(|e| e.to_string())
    }

    /// UV rectangle of a block's tile, as `(u0, v0, u1, v1)`.
    pub fn tile_uv(&self, tile: u32) -> [f32; 4] {
        let tx = (tile % ATLAS_COLUMNS) as f32;
        let ty = (tile / ATLAS_COLUMNS) as f32;
        let cols = ATLAS_COLUMNS as f32;
        let rows = (self.height / TILE) as f32;
        [tx / cols, ty / rows, (tx + 1.0) / cols, (ty + 1.0) / rows]
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let content_dir = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| deepcore_shared::DEFAULT_CONTENT_DIR.to_string());
    let out = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "assets/generated/atlas.png".to_string());
    let content = match Content::load_from_dir(&content_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("content error: {e}");
            std::process::exit(1);
        }
    };
    let atlas = build_atlas(&content);
    let path = PathBuf::from(&out);
    match atlas.save_png(&path) {
        Ok(()) => eprintln!(
            "wrote {} ({}x{}, {} tiles)",
            path.display(),
            atlas.width,
            atlas.height,
            atlas.tile_of_block.len()
        ),
        Err(e) => {
            eprintln!("failed to write atlas: {e}");
            std::process::exit(1);
        }
    }
}
