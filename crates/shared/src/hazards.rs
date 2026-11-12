//! Hazards: oxygen, body heat, radiation dose, flooding and cave-ins.

use serde::{Deserialize, Serialize};

use crate::content::Content;
use crate::content::defs::{HazardKind, Pattern};
use crate::voxel::chunk::{BlockPos, ChunkMap};

/// The player's vital statistics. Health and stamina are here too because hazards change them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vitals {
    /// Health, 0..=100.
    pub health: f32,
    /// Stamina, 0..=100.
    pub stamina: f32,
    /// Oxygen, 0..=100.
    pub oxygen: f32,
    /// Body heat in degrees Celsius above the comfort limit.
    pub body_heat: f32,
    /// Radiation dose, 0..=1000.
    pub dose: f32,
}

impl Default for Vitals {
    fn default() -> Self {
        Self {
            health: 100.0,
            stamina: 100.0,
            oxygen: 100.0,
            body_heat: 0.0,
            dose: 0.0,
        }
    }
}

impl Vitals {
    /// Whether the player is alive.
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }

    /// Screen grain intensity from radiation, 0..=1.
    pub fn grain(&self, content: &Content) -> f32 {
        let t = &content.tuning;
        if self.dose <= t.dose_grain {
            0.0
        } else {
            ((self.dose - t.dose_grain) / (t.dose_damage - t.dose_grain)).clamp(0.0, 1.0)
        }
    }
}

/// Protective gear worn by a player.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Protection {
    /// Radiation dose multiplier, 1.0 = unprotected.
    pub radiation: f32,
    /// Heat resistance multiplier, 1.0 = unprotected.
    pub heat: f32,
    /// Gas resistance multiplier, 1.0 = unprotected.
    pub gas: f32,
}

impl Default for Protection {
    fn default() -> Self {
        Self { radiation: 1.0, heat: 1.0, gas: 1.0 }
    }
}

/// The environmental sample around a player, produced by the server each tick.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Environment {
    /// Dose per second from nearby radioactive blocks and items.
    pub dose_per_sec: f32,
    /// Extra body heat per second from the ambient temperature.
    pub heat_per_sec: f32,
    /// Whether the player is submerged.
    pub underwater: bool,
    /// Whether the player is standing in a gas pocket.
    pub in_gas: bool,
    /// Ambient light level 0..=1, from the headlamp and placed lamps.
    pub light: f32,
}

/// Applies one tick of hazards to the vitals.
pub fn tick_vitals(
    content: &Content,
    vitals: &mut Vitals,
    environment: &Environment,
    protection: &Protection,
    dt: f32,
) {
    let t = &content.tuning;

    // Oxygen.
    if environment.underwater {
        vitals.oxygen = (vitals.oxygen - t.oxygen_drain * dt).max(0.0);
        if vitals.oxygen <= 0.0 {
            vitals.health -= 8.0 * dt;
        }
    } else if environment.in_gas {
        vitals.oxygen = (vitals.oxygen - t.oxygen_drain * 0.6 * protection.gas * dt).max(0.0);
        if vitals.oxygen <= 0.0 {
            vitals.health -= 6.0 * dt;
        }
    } else {
        vitals.oxygen = (vitals.oxygen + t.oxygen_refill * dt).min(100.0);
    }

    // Radiation.
    let dose_in = environment.dose_per_sec * protection.radiation;
    vitals.dose = (vitals.dose + dose_in * dt - t.dose_decay * dt * (1.0 - protection.radiation.min(1.0)))
        .clamp(0.0, 1000.0);
    if vitals.dose < dose_in {
        vitals.dose = (vitals.dose + dose_in * dt).clamp(0.0, 1000.0);
    }
    if vitals.dose > t.dose_damage {
        vitals.health -= t.radiation_dps * dt;
    }

    // Heat.
    let heat_in = environment.heat_per_sec * protection.heat;
    vitals.body_heat = (vitals.body_heat + (heat_in - 0.5) * dt).max(0.0);
    if vitals.body_heat > t.heat_limit {
        vitals.health -= t.heat_dps * dt;
    }

    // Regeneration: only when well fed, not irradiated and not overheating.
    if vitals.dose < t.dose_no_regen && vitals.body_heat < t.heat_limit && vitals.oxygen > 20.0 {
        vitals.health = (vitals.health + 0.4 * dt).min(100.0);
    }

    vitals.health = vitals.health.clamp(0.0, 100.0);
    vitals.stamina = vitals.stamina.clamp(0.0, 100.0);
}

/// Computes the radiation dose rate at a point from placed radioactive blocks.
///
/// Only blocks within their own radius contribute; the dose falls off linearly to zero at the
/// edge of the radius. This is deliberately cheap: the server samples a small cube around the
/// player instead of scanning the world.
pub fn dose_rate_at(
    content: &Content,
    map: &ChunkMap,
    pos: [f32; 3],
    max_radius: f32,
) -> f32 {
    let r = max_radius.ceil() as i32;
    let centre = BlockPos::from_world(pos);
    let mut dose = 0.0;
    for dy in -r..=r {
        for dz in -r..=r {
            for dx in -r..=r {
                let p = centre.offset(dx, dy, dz);
                let chunk_pos = crate::voxel::chunk::ChunkPos::from_block(p);
                let Some(chunk) = map.get(chunk_pos) else {
                    continue;
                };
                let block = chunk.get_index(p.chunk_index());
                if block == crate::content::ids::BlockId::AIR {
                    continue;
                }
                let Some(radiation) = content.block(block).def.radiation else {
                    continue;
                };
                let distance = p.distance_squared_to(pos).sqrt();
                if distance > radiation.radius {
                    continue;
                }
                let falloff = 1.0 - distance / radiation.radius;
                dose += radiation.dose_per_sec * falloff;
            }
        }
    }
    dose
}

/// The ambient heat rate at a depth, from the layer's heat hazard.
pub fn heat_rate_at(content: &Content, y: u32) -> f32 {
    let Some(layer) = content.layer_at_y(y) else {
        return 0.0;
    };
    let mut rate = 0.0;
    for hazard in &layer.hazards {
        if let HazardKind::Heat { rise } = hazard {
            rate += rise;
        }
    }
    // Deeper is hotter: scale linearly through the layer.
    let depth = content.tuning.surface_y.saturating_sub(y);
    let span = (layer.depth.1 - layer.depth.0).max(1);
    let through = (depth.saturating_sub(layer.depth.0)) as f32 / span as f32;
    rate * (0.5 + through)
}

/// The ambient radiation rate of a layer.
pub fn ambient_radiation(content: &Content, y: u32) -> f32 {
    let Some(layer) = content.layer_at_y(y) else {
        return 0.0;
    };
    layer
        .hazards
        .iter()
        .filter_map(|h| match h {
            HazardKind::Radiation { dose_per_sec } => Some(*dose_per_sec),
            _ => None,
        })
        .sum()
}

/// Whether a block is an exposed ore that emits radiation (used by the Geiger counter).
pub fn is_radioactive_ore(content: &Content, block: crate::content::ids::BlockId) -> bool {
    let def = &content.block(block).def;
    def.radiation.is_some() || matches!(def.pattern, Pattern::OreFleck(_))
}

/// Flood spread: returns the positions a water block at `origin` should spread to, limited by
/// `budget` blocks in total.
pub fn flood_spread(
    map: &ChunkMap,
    origin: BlockPos,
    budget: u32,
    is_water: impl Fn(BlockPos) -> bool,
) -> Vec<BlockPos> {
    let mut out = Vec::new();
    let mut frontier = vec![origin];
    let mut seen = vec![origin];
    while let Some(pos) = frontier.pop() {
        if out.len() as u32 >= budget {
            break;
        }
        for delta in [(0, -1, 0), (1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)] {
            let next = pos.offset(delta.0, delta.1, delta.2);
            if seen.contains(&next) {
                continue;
            }
            let chunk_pos = crate::voxel::chunk::ChunkPos::from_block(next);
            let Some(chunk) = map.get(chunk_pos) else {
                continue;
            };
            let block = chunk.get_index(next.chunk_index());
            if block == crate::content::ids::BlockId::AIR || is_water(next) {
                seen.push(next);
                frontier.push(next);
                out.push(next);
            }
        }
    }
    out
}

/// A cave-in check: whether mining `mined` would collapse the ceiling above it.
///
/// Returns the blocks that collapse.
pub fn cave_in_blocks(
    content: &Content,
    map: &ChunkMap,
    mined: BlockPos,
    max_span: u8,
) -> Vec<BlockPos> {
    // The ceiling is the first solid block above the mined one.
    let mut ceiling_y = mined.y + 1;
    let mut ceiling = None;
    while ceiling_y < mined.y + 8 {
        let p = BlockPos::new(mined.x, ceiling_y, mined.z);
        let chunk_pos = crate::voxel::chunk::ChunkPos::from_block(p);
        let Some(chunk) = map.get(chunk_pos) else {
            return Vec::new();
        };
        let block = chunk.get_index(p.chunk_index());
        if block != crate::content::ids::BlockId::AIR {
            ceiling = Some((p, block));
            break;
        }
        ceiling_y += 1;
    }
    let Some((ceiling_pos, _)) = ceiling else {
        return Vec::new();
    };
    // Measure the unsupported span in both horizontal axes.
    let span_x = unsupported_span(map, ceiling_pos, 1, 0);
    let span_z = unsupported_span(map, ceiling_pos, 0, 1);
    if span_x <= i32::from(max_span) && span_z <= i32::from(max_span) {
        return Vec::new();
    }
    // Collapse a shallow slab of the ceiling.
    let mut out = Vec::new();
    for dy in 0..2 {
        for dz in -1..=1 {
            for dx in -1..=1 {
                let p = ceiling_pos.offset(dx, dy, dz);
                let chunk_pos = crate::voxel::chunk::ChunkPos::from_block(p);
                if let Some(chunk) = map.get(chunk_pos) {
                    if chunk.get_index(p.chunk_index()) != crate::content::ids::BlockId::AIR {
                        out.push(p);
                    }
                }
            }
        }
    }
    let _ = content;
    out
}

fn unsupported_span(
    map: &ChunkMap,
    start: BlockPos,
    dx: i32,
    dz: i32,
) -> i32 {
    let solid = |p: BlockPos| {
        let chunk_pos = crate::voxel::chunk::ChunkPos::from_block(p);
        map.get(chunk_pos)
            .map(|c| c.get_index(p.chunk_index()) != crate::content::ids::BlockId::AIR)
            .unwrap_or(false)
    };
    let mut span = 1;
    for dir in [1, -1] {
        let mut p = start;
        for _ in 0..16 {
            p = p.offset(dx * dir, 0, dz * dir);
            if !solid(p) {
                break;
            }
            span += 1;
        }
    }
    span
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> Content {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/data");
        Content::load_from_dir(dir).expect("content loads")
    }

    #[test]
    fn radiation_damages_above_threshold() {
        let c = content();
        let mut vitals = Vitals::default();
        vitals.dose = 700.0;
        let env = Environment::default();
        for _ in 0..100 {
            tick_vitals(&c, &mut vitals, &env, &Protection::default(), 0.1);
        }
        assert!(vitals.health < 100.0, "health should drop: {}", vitals.health);
    }

    #[test]
    fn oxygen_drains_underwater() {
        let c = content();
        let mut vitals = Vitals::default();
        let env = Environment { underwater: true, ..Default::default() };
        tick_vitals(&c, &mut vitals, &env, &Protection::default(), 1.0);
        assert!(vitals.oxygen < 100.0);
    }

    #[test]
    fn hazmat_suit_reduces_dose() {
        let c = content();
        let env = Environment { dose_per_sec: 10.0, ..Default::default() };
        let mut bare = Vitals::default();
        let mut suited = Vitals::default();
        let suit = Protection { radiation: 0.2, ..Default::default() };
        for _ in 0..10 {
            tick_vitals(&c, &mut bare, &env, &Protection::default(), 0.1);
            tick_vitals(&c, &mut suited, &env, &suit, 0.1);
        }
        assert!(suited.dose < bare.dose * 0.5, "{} vs {}", suited.dose, bare.dose);
    }

    #[test]
    fn heat_rate_grows_with_depth() {
        let c = content();
        let shallow = heat_rate_at(&c, 300);
        let deep = heat_rate_at(&c, 170);
        assert!(deep > shallow, "{deep} should exceed {shallow}");
    }
}
