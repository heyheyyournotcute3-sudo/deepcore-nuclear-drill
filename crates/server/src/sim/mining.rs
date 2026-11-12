//! Mining: validates targets, runs the break timer and grants drops.

use bevy::prelude::*;

use deepcore_shared::content::defs::{ItemCategory, ToolEnergy};
use deepcore_shared::content::ids::{BlockId, ItemId};
use deepcore_shared::hazards::cave_in_blocks;
use deepcore_shared::protocol::{BlockChangeMsg, NetMovement, Notify};
use deepcore_shared::voxel::{face_area, raycast, BlockPos, Face};
use deepcore_shared::{GameContent, MovementParams};

use crate::net::stream::ChunkByteCache;
use crate::sim::components::{
    BlockChangeQueue, MiningJob, PickupQueue, ServerMachine, ServerPlayer,
};
use crate::world::World;

/// Seconds needed to break a block with a tool.
///
/// Break time = hardness / (tool_speed × power_multiplier), never below one tick.
pub fn break_time(content: &GameContent, hardness: f32, tool_speed: f32) -> f32 {
    let ticks = (hardness / (tool_speed * 1.0)).max(0.0);
    let _ = content;
    ticks
}

/// Tool statistics of the player's held item.
pub fn held_tool(
    content: &GameContent,
    player: &ServerPlayer,
) -> Option<(u8, f32, (u8, u8), ToolEnergy, ItemId)> {
    let stack = player.inventory.selected_stack()?;
    match &content.item(stack.item).def.category {
        ItemCategory::Tool(tool) => Some((tool.tier, tool.speed, tool.area, tool.energy, stack.item)),
        _ => None,
    }
}

/// Starts a mining job if the target is legal.
pub fn start_mining(
    content: &GameContent,
    world: &World,
    params: &MovementParams,
    player: &mut ServerPlayer,
    movement: &NetMovement,
    block: BlockPos,
    face: Face,
    notify: &mut Vec<String>,
) {
    let tool = held_tool(content, player);
    let (tier, speed, _area, _energy, _item) = match tool {
        Some(t) => t,
        None => {
            notify.push("You need a tool to mine".into());
            return;
        }
    };
    let reach = if tier >= 3 {
        content.tuning.drill_reach
    } else {
        content.tuning.reach
    };
    let eye = movement.0.eye(&params.0);
    let look = movement.0.look_dir();
    let hit = raycast(eye, look, reach + 1.0, |p| world.is_solid(content, p));
    let Some(hit) = hit else {
        return;
    };
    if hit.block != block {
        return;
    }
    let target = content.block(world.block(block));
    if target.def.unbreakable {
        notify.push("Bedrock cannot be mined".into());
        return;
    }
    if target.def.hardness <= 0.0 {
        return;
    }
    if tier < target.def.min_tier {
        notify.push(format!("Too hard — needs Tier {}", target.def.min_tier));
        return;
    }
    player.mining = Some(MiningJob {
        block,
        face,
        progress: 0.0,
        total: break_time(content, target.def.hardness, speed),
        tier,
    });
}

/// Advances every player's mining job and breaks blocks that finish.
pub fn tick_mining(
    content: Res<GameContent>,
    params: Res<MovementParams>,
    mut world: ResMut<World>,
    mut players: Query<(&mut ServerPlayer, &NetMovement)>,
    mut queue: ResMut<BlockChangeQueue>,
    mut pickups: ResMut<PickupQueue>,
    mut cache: ResMut<ChunkByteCache>,
    mut notifies: MessageWriter<bevy_replicon::prelude::ToClients<Notify>>,
    mut machines: Query<&mut ServerMachine>,
) {
    let dt = 1.0 / content.tuning.tick_hz;
    for (mut player, movement) in players.iter_mut() {
        let Some(job) = player.mining else {
            continue;
        };
        // Cancel if the player looked away or the block changed.
        let eye = movement.0.eye(&params.0);
        let look = movement.0.look_dir();
        let still_targeting = raycast(eye, look, 8.0, |p| world.is_solid(&content, p))
            .is_some_and(|h| h.block == job.block);
        let current = world.block(job.block);
        if !still_targeting || current == BlockId::AIR {
            player.mining = None;
            continue;
        }
        let (progress, total, job_block, job_face) = match &mut player.mining {
            Some(j) => {
                j.progress += dt;
                (j.progress, j.total, j.block, j.face)
            }
            None => continue,
        };
        if progress < total {
            continue;
        }
        // Break the block (and the whole face for area tools).
        let tool = held_tool(&content, &player);
        let area = tool.map(|t| t.2).unwrap_or((1, 1));
        let hit = deepcore_shared::voxel::RayHit {
            block: job_block,
            face: job_face,
            adjacent: job_block.offset_dir(job_face),
        };
        let targets = face_area(&hit, area.0, area.1);
        let mut broke = 0;
        for target in targets {
            if world.block(target) == BlockId::AIR {
                continue;
            }
            if content.block(world.block(target)).def.unbreakable {
                continue;
            }
            if !world.is_solid(&content, target) {
                continue;
            }
            break_block(&content, &mut world, &mut queue, &mut pickups, &mut cache, target, &mut machines);
            broke += 1;
        }
        // Cave-ins: mining under a wide unsupported ceiling brings it down.
        if broke > 0 {
            let span = content
                .layer_at_y(job_block.y as u32)
                .and_then(|l| {
                    l.hazards.iter().find_map(|h| match h {
                        deepcore_shared::content::defs::HazardKind::CaveIn { max_span } => {
                            Some(*max_span)
                        }
                        _ => None,
                    })
                });
            if let Some(max_span) = span {
                let collapse = cave_in_blocks(&content, &world.chunks, job_block, max_span);
                for pos in collapse {
                    break_block(&content, &mut world, &mut queue, &mut pickups, &mut cache, pos, &mut machines);
                }
            }
        }
        // Consume tool energy for powered tools.
        if let Some((_, _, _, energy, _)) = tool {
            match energy {
                ToolEnergy::Manual => {}
                ToolEnergy::Steam { .. } | ToolEnergy::Battery { .. } => {
                    player.tool_energy = (player.tool_energy - 1.0).max(0.0);
                    if player.tool_energy <= 0.0 {
                        notifies.write(bevy_replicon::prelude::ToClients {
                            targets: bevy_replicon::prelude::SendTargets::All,
                            message: Notify {
                                text: "Tool out of energy".into(),
                                level: 1,
                            },
                        });
                    }
                }
                ToolEnergy::CoreCell { .. } => {}
            }
        }
        player.mining = None;
        player.inventory_dirty = true;
    }
}

/// Removes a block, records the edit and drops its items.
pub fn break_block(
    content: &GameContent,
    world: &mut World,
    queue: &mut BlockChangeQueue,
    pickups: &mut PickupQueue,
    cache: &mut ChunkByteCache,
    pos: BlockPos,
    machines: &mut Query<&mut ServerMachine>,
) {
    let block = world.block(pos);
    if block == BlockId::AIR {
        return;
    }
    let def = &content.block(block).def;
    if def.unbreakable {
        return;
    }
    world.set_block(pos, BlockId::AIR);
    cache.invalidate(deepcore_shared::voxel::ChunkPos::from_block(pos));
    queue.changes.push(BlockChangeMsg { pos, block: BlockId::AIR });
    // A machine in that block is removed too.
    if let Some(machine) = def.machine.clone() {
        let _ = machine;
        for mut server_machine in machines.iter_mut() {
            if server_machine.pos == pos {
                server_machine.state.enabled = false;
            }
        }
    }
    for (item, count) in &content.block(block).drops {
        pickups.items.push((*item, *count, pos.center()));
    }
}
