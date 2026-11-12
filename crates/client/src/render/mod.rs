//! Chunk meshing and voxel rendering: greedy meshing on the async task pool, atlas material.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use deepcore_shared::content::ids::BlockId;
use deepcore_shared::voxel::{ChunkPos, CHUNK_SIZE};
use deepcore_shared::GameContent;

use crate::world::{ChunkEntities, ClientWorld};

/// A quad in a generated mesh.
#[derive(Debug, Clone, Copy)]
struct Quad {
    /// Four corner positions.
    corners: [[f32; 3]; 4],
    /// UV rectangle in the atlas.
    uv: [f32; 4],
    /// Face normal.
    normal: [f32; 3],
    /// Tint colour (linear RGB), used for ambient occlusion.
    color: [f32; 3],
}

/// Mesh data produced on a worker thread.
pub struct ChunkMesh {
    /// Vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// Vertex UVs.
    pub uvs: Vec<[f32; 2]>,
    /// Vertex colours.
    pub colors: Vec<[f32; 4]>,
    /// Triangle indices.
    pub indices: Vec<u32>,
}

/// Tracks in-flight and finished mesh jobs.
#[derive(Resource, Default)]
pub struct Mesher {
    /// Chunks waiting to be meshed.
    pub dirty: Vec<ChunkPos>,
    /// In-flight jobs.
    pub jobs: HashMap<ChunkPos, Task<ChunkMesh>>,
    /// Finished meshes waiting to be uploaded.
    pub ready: Vec<(ChunkPos, ChunkMesh)>,
}

/// The atlas texture and the UV rectangle of each block tile.
#[derive(Resource, Clone)]
pub struct Atlas {
    /// Atlas image.
    pub image: Handle<Image>,
    /// UV rectangle per block id.
    pub uv_of_block: Vec<[f32; 4]>,
}

/// The render plugin.
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mesher>()
            .add_systems(Startup, (setup_scene, build_atlas).chain())
            .add_systems(
                Update,
                (
                    submit_mesh_jobs,
                    collect_mesh_jobs,
                    apply_meshes,
                )
                    .chain(),
            );
    }
}

fn setup_scene(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        bevy::render::camera::CameraRenderGraph::new(bevy::core_pipeline::Core3d),
        Projection::Perspective(PerspectiveProjection {
            far: 400.0,
            ..default()
        }),
        Transform::from_xyz(512.0, 500.0, 512.0).looking_at(Vec3::new(512.0, 480.0, 512.0), Vec3::Y),
        crate::input::PlayerCamera,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.6, 0.0)),
        AmbientLight {
            color: Color::srgb(0.55, 0.62, 0.75),
            brightness: 380.0,
            ..default()
        },
    ));
}

/// Generates the atlas in memory at startup, so a fresh clone needs no assets on disk.
fn build_atlas(mut commands: Commands, mut images: ResMut<Assets<Image>>, content: Res<GameContent>) {
    let atlas = crate::render::atlas::build_atlas_image(&content);
    let uv_of_block = atlas.uv_of_block.clone();
    let image = images.add(atlas.image);
    commands.insert_resource(Atlas { image, uv_of_block });
}

/// Kicks off mesh jobs for dirty chunks, limited per frame to keep the frame time stable.
fn submit_mesh_jobs(
    mut mesher: ResMut<Mesher>,
    world: Res<ClientWorld>,
    atlas: Option<Res<Atlas>>,
    content: Res<GameContent>,
) {
    let Some(atlas) = atlas else {
        return;
    };
    let atlas = atlas.clone();
    let pool = AsyncComputeTaskPool::get();
    let mut started = 0;
    let dirty = std::mem::take(&mut mesher.dirty);
    for pos in dirty {
        if started >= 4 || mesher.jobs.contains_key(&pos) {
            continue;
        }
        let Some(chunk) = world.chunks.get(pos).cloned() else {
            continue;
        };
        let neighbours = NeighbourChunks {
            neg_x: world.chunks.get(ChunkPos::new(pos.x - 1, pos.y, pos.z)).cloned(),
            pos_x: world.chunks.get(ChunkPos::new(pos.x + 1, pos.y, pos.z)).cloned(),
            neg_y: world.chunks.get(ChunkPos::new(pos.x, pos.y - 1, pos.z)).cloned(),
            pos_y: world.chunks.get(ChunkPos::new(pos.x, pos.y + 1, pos.z)).cloned(),
            neg_z: world.chunks.get(ChunkPos::new(pos.x, pos.y, pos.z - 1)).cloned(),
            pos_z: world.chunks.get(ChunkPos::new(pos.x, pos.y, pos.z + 1)).cloned(),
        };
        let content_clone = content.0.clone();
        let atlas_clone = atlas.clone();
        let task = pool.spawn(async move {
            mesh_chunk(
                &chunk,
                &neighbours,
                &GameContent(content_clone),
                &atlas_clone,
            )
        });
        mesher.jobs.insert(pos, task);
        started += 1;
    }
}

fn collect_mesh_jobs(mut mesher: ResMut<Mesher>) {
    let done: Vec<ChunkPos> = mesher
        .jobs
        .iter()
        .filter(|(_, task)| task.is_finished())
        .map(|(pos, _)| *pos)
        .collect();
    for pos in done {
        if let Some(task) = mesher.jobs.remove(&pos) {
            let mesh = bevy::tasks::block_on(task);
            mesher.ready.push((pos, mesh));
        }
    }
}

fn apply_meshes(
    mut mesher: ResMut<Mesher>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    atlas: Option<Res<Atlas>>,
    mut entities: ResMut<ChunkEntities>,
    mut material_cache: Local<Option<Handle<StandardMaterial>>>,
) {
    let Some(atlas) = atlas else {
        return;
    };
    let material = material_cache
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color_texture: Some(atlas.image.clone()),
                perceptual_roughness: 0.92,
                metallic: 0.0,
                // The atlas is already sRGB, so no extra conversion is needed.
                ..default()
            })
        })
        .clone();

    for (pos, data) in std::mem::take(&mut mesher.ready) {
        if let Some(entity) = entities.map.get(&pos) {
            commands.entity(*entity).despawn();
        }
        if data.positions.is_empty() {
            entities.map.remove(&pos);
            continue;
        }
        let mut mesh = Mesh::new(
            bevy::render::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, data.uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, data.colors);
        mesh.insert_indices(bevy::render::mesh::Indices::U32(data.indices));
        let handle = meshes.add(mesh);
        let origin = pos.origin();
        let entity = commands
            .spawn((
                Mesh3d(handle),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(origin.x as f32, origin.y as f32, origin.z as f32),
            ))
            .id();
        entities.map.insert(pos, entity);
    }
}

/// Neighbouring chunks, used for face culling across chunk borders.
pub struct NeighbourChunks {
    /// Chunk at -X.
    pub neg_x: Option<deepcore_shared::Chunk>,
    /// Chunk at +X.
    pub pos_x: Option<deepcore_shared::Chunk>,
    /// Chunk at -Y.
    pub neg_y: Option<deepcore_shared::Chunk>,
    /// Chunk at +Y.
    pub pos_y: Option<deepcore_shared::Chunk>,
    /// Chunk at -Z.
    pub neg_z: Option<deepcore_shared::Chunk>,
    /// Chunk at +Z.
    pub pos_z: Option<deepcore_shared::Chunk>,
}

impl NeighbourChunks {
    /// The block adjacent to a local position in a direction, looking into the neighbour chunk.
    fn at(&self, x: i32, y: i32, z: i32, content: &GameContent) -> BlockId {
        let size = CHUNK_SIZE as i32;
        let (chunk, lx, ly, lz) = if x < 0 {
            (&self.neg_x, x + size, y, z)
        } else if x >= size {
            (&self.pos_x, x - size, y, z)
        } else if y < 0 {
            (&self.neg_y, x, y + size, z)
        } else if y >= size {
            (&self.pos_y, x, y - size, z)
        } else if z < 0 {
            (&self.neg_z, x, y, z + size)
        } else if z >= size {
            (&self.pos_z, x, y, z - size)
        } else {
            return BlockId::AIR;
        };
        match chunk {
            Some(chunk) => chunk.get_local(lx as u32, ly as u32, lz as u32),
            None => {
                let _ = content;
                BlockId::AIR
            }
        }
    }
}

/// Builds a greedy mesh for one chunk.
///
/// Faces between two opaque blocks are culled, and coplanar faces of the same block are merged
/// into larger quads.
pub fn mesh_chunk(
    chunk: &deepcore_shared::Chunk,
    neighbours: &NeighbourChunks,
    content: &GameContent,
    atlas: &Atlas,
) -> ChunkMesh {
    let mut mesh = ChunkMesh {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
    };
    let size = CHUNK_SIZE as i32;
    let solid = |x: i32, y: i32, z: i32| -> bool {
        let block = if (0..size).contains(&x) && (0..size).contains(&y) && (0..size).contains(&z) {
            chunk.get_local(x as u32, y as u32, z as u32)
        } else {
            neighbours.at(x, y, z, content)
        };
        let def = &content.block(block).def;
        def.solid
    };

    for face in 0..6 {
        let (axis, sign) = match face {
            0 => (0, -1),
            1 => (0, 1),
            2 => (1, -1),
            3 => (1, 1),
            4 => (2, -1),
            _ => (2, 1),
        };
        let u_axis = (axis + 1) % 3;
        let v_axis = (axis + 2) % 3;
        let mut mask = vec![BlockId::AIR; (CHUNK_SIZE * CHUNK_SIZE) as usize];
        let mut merged = vec![false; (CHUNK_SIZE * CHUNK_SIZE) as usize];

        for slice in 0..size {
            // Build the face mask for this slice.
            for v in 0..size {
                for u in 0..size {
                    let mut pos = [0i32; 3];
                    pos[axis] = slice;
                    pos[u_axis] = u;
                    pos[v_axis] = v;
                    let block = chunk.get_local(pos[0] as u32, pos[1] as u32, pos[2] as u32);
                    let def = &content.block(block).def;
                    if !def.solid {
                        mask[(u + v * size) as usize] = BlockId::AIR;
                        continue;
                    }
                    let mut npos = pos;
                    npos[axis] += sign;
                    if solid(npos[0], npos[1], npos[2]) {
                        mask[(u + v * size) as usize] = BlockId::AIR;
                    } else {
                        mask[(u + v * size) as usize] = block;
                    }
                }
            }
            merged.iter_mut().for_each(|m| *m = false);

            // Greedy-merge the mask into quads.
            for v in 0..size {
                for u in 0..size {
                    let idx = (u + v * size) as usize;
                    let block = mask[idx];
                    if block == BlockId::AIR || merged[idx] {
                        continue;
                    }
                    // Extend along u.
                    let mut width = 1;
                    while u + width < size {
                        let i = (u + width + v * size) as usize;
                        if mask[i] != block || merged[i] {
                            break;
                        }
                        width += 1;
                    }
                    // Extend along v.
                    let mut height = 1;
                    'grow: while v + height < size {
                        for du in 0..width {
                            let i = (u + du + (v + height) * size) as usize;
                            if mask[i] != block || merged[i] {
                                break 'grow;
                            }
                        }
                        height += 1;
                    }
                    for dv in 0..height {
                        for du in 0..width {
                            merged[(u + du + (v + dv) * size) as usize] = true;
                        }
                    }
                    push_quad(
                        &mut mesh,
                        block,
                        axis,
                        sign,
                        slice,
                        u,
                        v,
                        width,
                        height,
                        content,
                        atlas,
                    );
                }
            }
        }
    }
    mesh
}

#[allow(clippy::too_many_arguments)]
fn push_quad(
    mesh: &mut ChunkMesh,
    block: BlockId,
    axis: usize,
    sign: i32,
    slice: i32,
    u: i32,
    v: i32,
    width: i32,
    height: i32,
    content: &GameContent,
    atlas: &Atlas,
) {
    let u_axis = (axis + 1) % 3;
    let v_axis = (axis + 2) % 3;
    let plane = if sign > 0 { slice as f32 + 1.0 } else { slice as f32 };

    let corner = |du: i32, dv: i32| -> [f32; 3] {
        let mut p = [0.0f32; 3];
        p[axis] = plane;
        p[u_axis] = (u + du) as f32;
        p[v_axis] = (v + dv) as f32;
        p
    };

    let mut corners = [
        corner(0, 0),
        corner(width, 0),
        corner(width, height),
        corner(0, height),
    ];
    if sign < 0 {
        corners.swap(1, 3);
    }

    let mut normal = [0.0f32; 3];
    normal[axis] = sign as f32;

    let uv = atlas
        .uv_of_block
        .get(block.0 as usize)
        .copied()
        .unwrap_or([0.0, 0.0, 0.0625, 0.0625]);

    // UVs repeat over the merged quad so the texture does not stretch.
    let tile = [uv[2] - uv[0], uv[3] - uv[1]];
    let uvs = [
        [uv[0], uv[1]],
        [uv[0] + tile[0] * width as f32, uv[1]],
        [uv[0] + tile[0] * width as f32, uv[1] + tile[1] * height as f32],
        [uv[0], uv[1] + tile[1] * height as f32],
    ];

    // Cheap ambient occlusion: faces pointing up are bright, down dark, sides medium.
    let emissive = content.block(block).def.emissive.clamp(0.0, 1.0);
    let shade = match axis {
        1 if sign > 0 => 1.0,
        1 => 0.45,
        0 => 0.72,
        _ => 0.86,
    };
    let tint = [shade, shade, shade];
    let _ = emissive;

    let base = mesh.positions.len() as u32;
    for i in 0..4 {
        mesh.positions.push(corners[i]);
        mesh.normals.push(normal);
        mesh.uvs.push(uvs[i]);
        mesh.colors.push([tint[0], tint[1], tint[2], 1.0]);
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Builds the atlas image and UV table. Re-exported from `atlas`.
pub mod atlas {
    use bevy::prelude::*;

    use deepcore_shared::GameContent;

    /// The atlas image plus the UV rectangle of each block.
    pub struct AtlasData {
        /// The RGBA image.
        pub image: Image,
        /// UV rectangles per block id.
        pub uv_of_block: Vec<[f32; 4]>,
    }

    /// Tile size in pixels.
    pub const TILE: u32 = 16;
    /// Tiles per atlas row.
    pub const COLUMNS: u32 = 16;

    /// Paints the atlas for a content set in memory.
    pub fn build_atlas_image(content: &GameContent) -> AtlasData {
        let count = content.blocks.len() as u32;
        let rows = count.div_ceil(COLUMNS).max(1);
        let width = COLUMNS * TILE;
        let height = rows * TILE;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut uv_of_block = Vec::with_capacity(content.blocks.len());

        for (index, block) in content.blocks.iter().enumerate() {
            let tile = index as u32;
            let tx = (tile % COLUMNS) * TILE;
            let ty = (tile / COLUMNS) * TILE;
            let cols = COLUMNS as f32;
            let row_count = rows as f32;
            uv_of_block.push([
                tx as f32 / width as f32,
                ty as f32 / height as f32,
                (tx + TILE) as f32 / width as f32,
                (ty + TILE) as f32 / height as f32,
            ]);
            let _ = cols;
            let _ = row_count;
            let def = &block.def;
            let [r, g, b] = def.color.to_array();
            for py in 0..TILE {
                for px in 0..TILE {
                    let (cr, cg, cb) = tile_color(def, px, py, tile, r, g, b);
                    let offset = (((ty + py) * width + tx + px) * 4) as usize;
                    pixels[offset] = linear_to_srgb(cr);
                    pixels[offset + 1] = linear_to_srgb(cg);
                    pixels[offset + 2] = linear_to_srgb(cb);
                    pixels[offset + 3] = 255;
                }
            }
        }

        let image = Image::new(
            bevy::render::render_resource::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            pixels,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        );
        AtlasData { image, uv_of_block }
    }

    fn hash(x: u32, y: u32, salt: u32) -> u32 {
        let mut h = x.wrapping_mul(0x1F1F_1F1F) ^ y.wrapping_mul(0x9E37_79B9) ^ salt;
        h ^= h >> 15;
        h = h.wrapping_mul(0x85EB_CA6B);
        h ^= h >> 13;
        h
    }

    fn tile_color(
        def: &deepcore_shared::content::defs::BlockDef,
        px: u32,
        py: u32,
        tile: u32,
        r: f32,
        g: f32,
        b: f32,
    ) -> (f32, f32, f32) {
        use deepcore_shared::content::defs::Pattern;
        let n = (hash(px, py, tile) % 1000) as f32 / 1000.0;
        let (mut cr, mut cg, mut cb) = (r, g, b);
        match def.pattern {
            Pattern::Solid => {
                let s = 0.94 + n * 0.12;
                cr *= s;
                cg *= s;
                cb *= s;
            }
            Pattern::Speckle => {
                let s = 0.78 + n * 0.44;
                cr *= s;
                cg *= s;
                cb *= s;
            }
            Pattern::Bands => {
                let s = (if (py / 3) % 2 == 0 { 1.08 } else { 0.9 }) * (0.94 + n * 0.12);
                cr *= s;
                cg *= s;
                cb *= s;
            }
            Pattern::Cracks => {
                let crack = (px * 7 + py * 13 + hash(px / 4, py / 4, tile) % 17) % 23 == 0;
                let s = if crack { 0.55 } else { 0.86 + n * 0.24 };
                cr *= s;
                cg *= s;
                cb *= s;
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
                let s = 0.78 + n * 0.4;
                cr *= s;
                cg *= s;
                cb *= s;
                if hash(px / 3, py / 3, tile ^ 0xABCD) % 100 < 26 {
                    cr = fleck.0;
                    cg = fleck.1;
                    cb = fleck.2;
                }
            }
        }
        let e = def.emissive.clamp(0.0, 1.0);
        (
            (cr + e * 0.35).min(1.0),
            (cg + e * 0.35).min(1.0),
            (cb + e * 0.35).min(1.0),
        )
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
}
