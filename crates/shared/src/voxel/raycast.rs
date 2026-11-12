//! Voxel ray casting (Amanatides–Woo DDA) used for block targeting, line of sight and
//! ore scanning. Pure and deterministic, so both sides compute the same hit.

use crate::content::ids::BlockId;
use crate::voxel::chunk::{BlockPos, Face};

/// The result of a ray cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RayHit {
    /// The block that was hit.
    pub block: BlockPos,
    /// The face that was hit.
    pub face: Face,
    /// The empty block in front of the hit face, where a placed block would go.
    pub adjacent: BlockPos,
}

/// Casts a ray through the voxel grid.
///
/// `is_solid` reports whether a block should stop the ray. Returns the first solid block within
/// `max_distance`, or `None`.
pub fn raycast(
    origin: [f32; 3],
    direction: [f32; 3],
    max_distance: f32,
    is_solid: impl Fn(BlockPos) -> bool,
) -> Option<RayHit> {
    let mut x = origin[0].floor() as i32;
    let mut y = origin[1].floor() as i32;
    let mut z = origin[2].floor() as i32;

    let len = (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2])
        .sqrt();
    if len <= f32::EPSILON {
        return None;
    }
    let dx = direction[0] / len;
    let dy = direction[1] / len;
    let dz = direction[2] / len;

    let step_x = if dx > 0.0 { 1 } else if dx < 0.0 { -1 } else { 0 };
    let step_y = if dy > 0.0 { 1 } else if dy < 0.0 { -1 } else { 0 };
    let step_z = if dz > 0.0 { 1 } else if dz < 0.0 { -1 } else { 0 };

    let t_delta_x = if step_x == 0 { f32::INFINITY } else { (1.0 / dx).abs() };
    let t_delta_y = if step_y == 0 { f32::INFINITY } else { (1.0 / dy).abs() };
    let t_delta_z = if step_z == 0 { f32::INFINITY } else { (1.0 / dz).abs() };

    let mut t_max_x = if step_x == 0 {
        f32::INFINITY
    } else if step_x > 0 {
        (x as f32 + 1.0 - origin[0]) * t_delta_x
    } else {
        (origin[0] - x as f32) * t_delta_x
    };
    let mut t_max_y = if step_y == 0 {
        f32::INFINITY
    } else if step_y > 0 {
        (y as f32 + 1.0 - origin[1]) * t_delta_y
    } else {
        (origin[1] - y as f32) * t_delta_y
    };
    let mut t_max_z = if step_z == 0 {
        f32::INFINITY
    } else if step_z > 0 {
        (z as f32 + 1.0 - origin[2]) * t_delta_z
    } else {
        (origin[2] - z as f32) * t_delta_z
    };

    let mut face;
    let mut travelled = 0.0f32;

    // Start inside a solid block: report it with the face opposite the ray direction.
    if is_solid(BlockPos::new(x, y, z)) {
        let f = Face::facing([dx, dy, dz]);
        return Some(RayHit {
            block: BlockPos::new(x, y, z),
            face: f,
            adjacent: BlockPos::new(x, y, z).offset_dir(f),
        });
    }

    while travelled <= max_distance {
        if t_max_x < t_max_y && t_max_x < t_max_z {
            travelled = t_max_x;
            x += step_x;
            t_max_x += t_delta_x;
            face = if step_x > 0 { Face::NegX } else { Face::PosX };
        } else if t_max_y < t_max_z {
            travelled = t_max_y;
            y += step_y;
            t_max_y += t_delta_y;
            face = if step_y > 0 { Face::NegY } else { Face::PosY };
        } else {
            travelled = t_max_z;
            z += step_z;
            t_max_z += t_delta_z;
            face = if step_z > 0 { Face::NegZ } else { Face::PosZ };
        }
        if travelled > max_distance {
            break;
        }
        let pos = BlockPos::new(x, y, z);
        if is_solid(pos) {
            return Some(RayHit {
                block: pos,
                face,
                adjacent: pos.offset_dir(face),
            });
        }
    }
    None
}

/// Whether a straight line between two points passes through any solid block.
pub fn line_of_sight(
    from: [f32; 3],
    to: [f32; 3],
    is_solid: impl Fn(BlockPos) -> bool,
) -> bool {
    let dir = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let dist = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if dist <= f32::EPSILON {
        return true;
    }
    // Stop just short of the target so the target block itself does not block the check.
    match raycast(from, dir, (dist - 0.05).max(0.0), is_solid) {
        None => true,
        Some(hit) => hit.block == BlockPos::from_world(to),
    }
}

/// Every block in a `width × height` face perpendicular to the targeted face, centred on `hit`.
///
/// Used by area-mining tools such as the Nuclear Drill's 3×3 face.
pub fn face_area(hit: &RayHit, width: u8, height: u8) -> Vec<BlockPos> {
    if width <= 1 && height <= 1 {
        return vec![hit.block];
    }
    // Pick the two axes that are not the face normal.
    let (axis_u, axis_v) = match hit.face {
        Face::NegX | Face::PosX => ((0, 1, 0), (0, 0, 1)),
        Face::NegY | Face::PosY => ((1, 0, 0), (0, 0, 1)),
        Face::NegZ | Face::PosZ => ((1, 0, 0), (0, 1, 0)),
    };
    let half_w = i32::from(width) / 2;
    let half_h = i32::from(height) / 2;
    let mut out = Vec::with_capacity(usize::from(width) * usize::from(height));
    for du in -half_w..=half_w {
        for dv in -half_h..=half_h {
            out.push(BlockPos::new(
                hit.block.x + axis_u.0 * du + axis_v.0 * dv,
                hit.block.y + axis_u.1 * du + axis_v.1 * dv,
                hit.block.z + axis_u.2 * du + axis_v.2 * dv,
            ));
        }
    }
    out
}

/// Finds all positions within `radius` of `centre` whose block satisfies `predicate`.
/// Used by the Ore Scanner; returns positions sorted by distance.
pub fn scan_radius(
    centre: BlockPos,
    radius: i32,
    mut predicate: impl FnMut(BlockPos) -> bool,
) -> Vec<BlockPos> {
    let mut found = Vec::new();
    for dy in -radius..=radius {
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy + dz * dz > radius * radius {
                    continue;
                }
                let pos = centre.offset(dx, dy, dz);
                if predicate(pos) {
                    found.push(pos);
                }
            }
        }
    }
    let c = centre.center();
    found.sort_by(|a, b| {
        a.distance_squared_to(c)
            .partial_cmp(&b.distance_squared_to(c))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    found
}

/// A convenience trait for looking up blocks in a generic container.
pub trait BlockLookup {
    /// Returns the block at a position, or air outside the world.
    fn block_at(&self, pos: BlockPos) -> BlockId;
}

impl BlockLookup for ChunkLookup<'_> {
    fn block_at(&self, pos: BlockPos) -> BlockId {
        self.get(pos)
    }
}

/// A borrowed view over a chunk map, used to answer block lookups.
#[derive(Clone, Copy)]
pub struct ChunkLookup<'a> {
    map: &'a crate::voxel::chunk::ChunkMap,
}

impl<'a> ChunkLookup<'a> {
    /// Creates a lookup over a chunk map.
    pub fn new(map: &'a crate::voxel::chunk::ChunkMap) -> Self {
        Self { map }
    }

    /// Returns the block at a position, or air if the chunk is not loaded.
    pub fn get(&self, pos: BlockPos) -> BlockId {
        let chunk_pos = crate::voxel::chunk::ChunkPos::from_block(pos);
        match self.map.get(chunk_pos) {
            None => BlockId::AIR,
            Some(chunk) => chunk.get_index(pos.chunk_index()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_block_in_front() {
        let hit = raycast([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 10.0, |p| {
            p == BlockPos::new(3, 0, 0)
        })
        .expect("hit");
        assert_eq!(hit.block, BlockPos::new(3, 0, 0));
        assert_eq!(hit.face, Face::NegX);
        assert_eq!(hit.adjacent, BlockPos::new(2, 0, 0));
    }

    #[test]
    fn ray_misses_when_out_of_range() {
        let hit = raycast([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 2.0, |p| {
            p == BlockPos::new(3, 0, 0)
        });
        assert!(hit.is_none());
    }

    #[test]
    fn area_face_is_centred() {
        let hit = RayHit {
            block: BlockPos::new(5, 5, 5),
            face: Face::PosY,
            adjacent: BlockPos::new(5, 6, 5),
        };
        let cells = face_area(&hit, 3, 3);
        assert_eq!(cells.len(), 9);
        assert!(cells.contains(&BlockPos::new(5, 5, 5)));
        assert!(cells.contains(&BlockPos::new(4, 5, 4)));
        assert!(cells.contains(&BlockPos::new(6, 5, 6)));
    }

    #[test]
    fn line_of_sight_through_air() {
        let los = line_of_sight([0.5, 0.5, 0.5], [8.5, 0.5, 0.5], |_| false);
        assert!(los);
    }
}
