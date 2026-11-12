//! Machine placement, storage, upgrades and machine-to-machine logistics.

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::content::defs::{EnergyKind, MachineRole};
use deepcore_shared::crafting::MachineState;
use deepcore_shared::protocol::{BlockChangeMsg, NetMachine};
use deepcore_shared::voxel::BlockPos;
use deepcore_shared::GameContent;

use crate::sim::components::{BlockChangeQueue, MachineIndex, ServerMachine};
use crate::world::World;

/// Spawns the block entity for a machine that was just placed.
///
/// Returns the machine entity, or `None` if the machine needs more space than is free.
pub fn try_place_machine(
    commands: &mut Commands,
    content: &GameContent,
    world: &mut World,
    index: &mut MachineIndex,
    machine_id: deepcore_shared::content::ids::MachineId,
    pos: BlockPos,
    facing: u8,
    queue: &mut BlockChangeQueue,
) -> Option<Entity> {
    let def = content.machine(machine_id).def.clone();
    let block = content.machine(machine_id).block?;
    // The footprint must be free: every cell either air or the origin we just filled.
    for dx in 0..i32::from(def.size.0) {
        for dy in 0..i32::from(def.size.1) {
            for dz in 0..i32::from(def.size.2) {
                let cell = pos.offset(dx, dy, dz);
                if cell == pos {
                    continue;
                }
                if world.block(cell) != deepcore_shared::voxel::BlockId::AIR {
                    return None;
                }
            }
        }
    }
    // Fill the footprint so the machine is solid and cannot be walked through.
    for dx in 0..i32::from(def.size.0) {
        for dy in 0..i32::from(def.size.1) {
            for dz in 0..i32::from(def.size.2) {
                let cell = pos.offset(dx, dy, dz);
                if cell != pos {
                    world.set_block(cell, block);
                    queue.changes.push(BlockChangeMsg { pos: cell, block });
                }
            }
        }
    }

    let state = MachineState::new(content, machine_id);
    let entity = commands
        .spawn((
            ServerMachine {
                machine: machine_id,
                pos,
                facing,
                state,
                network_pos: None,
                core_temp: 20.0,
                rod_insertion: 1.0,
                reactor_kw: 0.0,
                scrameed: false,
            },
            NetMachine {
                machine: machine_id,
                pos,
                progress: 0.0,
                running: false,
                facing,
            },
            Transform::from_xyz(pos.x as f32, pos.y as f32, pos.z as f32),
        ))
        .id();
    commands.entity(entity).insert(Replicated);
    index.by_pos.insert(pos, entity);
    Some(entity)
}

/// Removes a machine entity when its block is mined.
pub fn remove_machine_at(
    commands: &mut Commands,
    index: &mut MachineIndex,
    pos: BlockPos,
) -> bool {
    match index.by_pos.remove(&pos) {
        Some(entity) => {
            commands.entity(entity).despawn();
            true
        }
        None => false,
    }
}

/// Machines that need a network connection: boilers, engines, generators, batteries.
pub fn network_kind(content: &GameContent, machine: deepcore_shared::content::ids::MachineId) -> Option<deepcore_shared::power::NetworkKind> {
    let def = &content.machine(machine).def;
    match def.role {
        MachineRole::Boiler { .. } | MachineRole::SteamEngine { .. } => {
            Some(deepcore_shared::power::NetworkKind::Steam)
        }
        MachineRole::Generator { .. } | MachineRole::Battery { .. } | MachineRole::Reactor => {
            Some(deepcore_shared::power::NetworkKind::Electric)
        }
        _ => match def.energy {
            EnergyKind::Steam { .. } => Some(deepcore_shared::power::NetworkKind::Steam),
            EnergyKind::Electric { .. } => Some(deepcore_shared::power::NetworkKind::Electric),
            _ => None,
        },
    }
}

/// Whether a machine's outputs should be pushed into an adjacent container.
pub fn is_container(content: &GameContent, machine: deepcore_shared::content::ids::MachineId) -> bool {
    content.machine(machine).def.role == MachineRole::Storage
}

/// A conveyor's throughput: items moved per second to the next machine in front of it.
pub const CONVEYOR_ITEMS_PER_SEC: f32 = 2.0;

/// The block position in front of a facing direction (0 = -Z, 1 = +X, 2 = +Z, 3 = -X).
pub fn facing_offset(pos: BlockPos, facing: u8) -> BlockPos {
    match facing % 4 {
        0 => pos.offset(0, 0, -1),
        1 => pos.offset(1, 0, 0),
        2 => pos.offset(0, 0, 1),
        _ => pos.offset(-1, 0, 0),
    }
}

/// The machine at a position, if any.
pub fn machine_entity_at(index: &MachineIndex, pos: BlockPos) -> Option<Entity> {
    index.by_pos.get(&pos).copied()
}

/// Moves items from one machine's outputs into another machine's inputs.
pub fn transfer_between(
    content: &GameContent,
    from: &mut ServerMachine,
    to: &mut ServerMachine,
) -> u16 {
    let mut moved = 0;
    for slot in &mut from.state.outputs {
        let Some(stack) = &mut slot.stack else {
            continue;
        };
        let item = stack.item;
        let max = deepcore_shared::inventory::Inventory::max_stack(content, item);
        let mut remaining = stack.count;
        for target in &mut to.state.inputs {
            if remaining == 0 {
                break;
            }
            match &mut target.stack {
                None => {
                    let take = max.min(remaining);
                    target.stack = Some(deepcore_shared::inventory::ItemStack::new(item, take));
                    remaining -= take;
                    moved += take;
                }
                Some(existing) if existing.item == item && existing.count < max => {
                    let take = (max - existing.count).min(remaining);
                    existing.count += take;
                    remaining -= take;
                    moved += take;
                }
                _ => {}
            }
        }
        stack.count = remaining;
        if stack.count == 0 {
            slot.stack = None;
        }
    }
    moved
}

/// Moves items from a player's inventory into a machine's input slots.
pub fn insert_into_machine(
    content: &GameContent,
    inventory: &mut deepcore_shared::inventory::Inventory,
    machine: &mut ServerMachine,
    slot: u16,
    count: u16,
) -> u16 {
    let Some(stack) = inventory.slots.get(usize::from(slot)).and_then(|s| s.stack) else {
        return 0;
    };
    let item = stack.item;
    let max = deepcore_shared::inventory::Inventory::max_stack(content, item);
    let mut remaining = stack.count.min(count);
    let mut moved = 0;
    for target in machine
        .state
        .inputs
        .iter_mut()
        .chain(machine.state.fuel.iter_mut())
    {
        if remaining == 0 {
            break;
        }
        match &mut target.stack {
            None => {
                let take = max.min(remaining);
                target.stack = Some(deepcore_shared::inventory::ItemStack::new(item, take));
                remaining -= take;
                moved += take;
            }
            Some(existing) if existing.item == item && existing.count < max => {
                let take = (max - existing.count).min(remaining);
                existing.count += take;
                remaining -= take;
                moved += take;
            }
            _ => {}
        }
    }
    if moved > 0 {
        let _ = inventory.take_from_slot(usize::from(slot), moved);
    }
    moved
}

/// Moves items from a machine's output slots into a player's inventory.
pub fn take_from_machine(
    content: &GameContent,
    machine: &mut ServerMachine,
    inventory: &mut deepcore_shared::inventory::Inventory,
    slot: u8,
    count: u16,
) -> u16 {
    let Some(target) = machine.state.outputs.get_mut(usize::from(slot)) else {
        return 0;
    };
    let Some(stack) = target.stack else {
        return 0;
    };
    let take = stack.count.min(count);
    let result = inventory.insert(content, stack.item, take);
    let stored = take - result.leftover;
    if stored > 0 {
        if let Some(existing) = &mut target.stack {
            existing.count -= stored;
            if existing.count == 0 {
                target.stack = None;
            }
        }
    }
    stored
}

/// Runs conveyors: each conveyor pushes items from the machine behind it into the machine in
/// front of it, at a fixed throughput.
pub fn tick_logistics(
    content: Res<GameContent>,
    mut machines: Query<(Entity, &mut ServerMachine)>,
    index: Res<MachineIndex>,
    mut accumulator: Local<f32>,
    time: Res<Time<Fixed>>,
) {
    *accumulator += time.delta_secs();
    let interval = 1.0 / CONVEYOR_ITEMS_PER_SEC;
    if *accumulator < interval {
        return;
    }
    *accumulator -= interval;

    // Collect the pairs first to avoid aliasing two mutable borrows at once.
    let mut pairs: Vec<(Entity, Entity)> = Vec::new();
    for (entity, machine) in machines.iter() {
        let ahead = facing_offset(machine.pos, machine.facing);
        if let Some(target) = machine_entity_at(&index, ahead) {
            pairs.push((entity, target));
        }
    }
    for (from, to) in pairs {
        let Ok([(_, mut a), (_, mut b)]) = machines.get_many_mut([from, to]) else {
            continue;
        };
        transfer_between(&content, &mut a, &mut b);
    }
}
