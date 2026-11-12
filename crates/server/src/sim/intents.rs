//! Intent handling: validates every client request against the authoritative world.

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::content::defs::{ItemCategory, MachineRole};
use deepcore_shared::content::ids::{BlockId, ItemId, RecipeId, TechId};
use deepcore_shared::crafting::{find_craftable, tick_processor, MachineState, TickOutcome};
use deepcore_shared::inventory::{Inventory, Slot};
use deepcore_shared::protocol::{
    BlockChangeMsg, Intent, InventoryUpdate, MachineContents, NetHeld, NetMachine, NetMovement,
    Notify, ScannerResult,
};
use deepcore_shared::voxel::{line_of_sight, raycast, BlockPos};
use deepcore_shared::{GameContent, MovementParams};

use crate::net::stream::ChunkByteCache;
use crate::sim::components::*;
use crate::sim::mining::{break_block, start_mining};
use crate::world::World;

/// Processes all intents received this tick.
#[allow(clippy::too_many_arguments)]
pub fn handle_intents(
    mut intents: MessageReader<FromClient<Intent>>,
    content: Res<GameContent>,
    params: Res<MovementParams>,
    mut world: ResMut<World>,
    mut players: Query<(&mut ServerPlayer, &mut NetMovement, &mut NetHeld)>,
    mut machines: Query<&mut ServerMachine>,
    mut research: ResMut<Research>,
    mut queue: ResMut<BlockChangeQueue>,
    mut pickups: ResMut<PickupQueue>,
    mut cache: ResMut<ChunkByteCache>,
    mut notifies: MessageWriter<ToClients<Notify>>,
    mut contents: MessageWriter<ToClients<MachineContents>>,
    mut scanners: MessageWriter<ToClients<ScannerResult>>,
    sessions: Query<(Entity, &crate::net::handshake::ClientSession)>,
    mut machine_index: ResMut<MachineIndex>,
    mut commands: Commands,
) {
    for FromClient { client_id, message } in intents.read() {
        let Some(client_entity) = client_id.entity() else {
            continue;
        };
        let Some(player_entity) = sessions
            .iter()
            .find(|(e, _)| *e == client_entity)
            .and_then(|(_, s)| s.player)
        else {
            continue;
        };
        let Ok((mut player, mut movement, mut held)) = players.get_mut(player_entity) else {
            continue;
        };
        let mut out = Vec::new();
        match message {
            Intent::Move(input) => {
                if player.pending_inputs.len() < 32 {
                    player.pending_inputs.push(*input);
                }
            }
            Intent::StartMining { block, face } => {
                start_mining(
                    &content,
                    &world,
                    &params,
                    &mut player,
                    &movement,
                    *block,
                    *face,
                    &mut out,
                );
            }
            Intent::StopMining => {
                player.mining = None;
            }
            Intent::PlaceBlock { pos, face, slot, facing } => {
                place_block(
                    &mut commands,
                    &content,
                    &mut world,
                    &params,
                    &mut player,
                    &movement,
                    &mut machines,
                    &mut queue,
                    &mut cache,
                    &mut machine_index,
                    *pos,
                    *face,
                    *slot,
                    *facing,
                    &mut out,
                );
            }
            Intent::Interact { pos } => {
                interact(
                    &content,
                    &params,
                    &mut player,
                    &movement,
                    &mut machines,
                    *pos,
                    &mut out,
                );
            }
            Intent::MoveItems { from, to, count } => {
                let result = player
                    .inventory
                    .move_between(&content, usize::from(*from), usize::from(*to), *count);
                if result.is_err() {
                    out.push("Cannot move those items".into());
                }
                player.inventory_dirty = true;
            }
            Intent::QuickMove { slot } => {
                let _ = player.inventory.quick_move(&content, usize::from(*slot));
                player.inventory_dirty = true;
            }
            Intent::DropItem { slot, count } => {
                if let Ok(stack) = player.inventory.take_from_slot(usize::from(*slot), *count) {
                    let p = movement.0.position;
                    pickups.items.push((stack.item, stack.count, [p[0], p[1] + 1.0, p[2]]));
                    player.inventory_dirty = true;
                }
            }
            Intent::SelectSlot { slot } => {
                player.inventory.select(*slot);
                player.inventory_dirty = true;
            }
            Intent::Craft { recipe, count } => {
                craft(&content, &mut player, *recipe, *count, &mut out);
            }
            Intent::MachineTransfer { machine, from_player, from_machine, to_machine, to_player, count } => {
                machine_transfer(
                    &content,
                    &mut player,
                    &mut machines,
                    *machine,
                    *from_player,
                    *from_machine,
                    *to_machine,
                    *to_player,
                    *count,
                    &mut out,
                );
            }
            Intent::SetMachineRecipe { machine, recipe } => {
                set_machine_recipe(&content, &research, &mut machines, *machine, *recipe, &mut out);
            }
            Intent::ResearchDeposit { node } => {
                deposit_research(&content, &mut player, &mut research, *node, &mut out);
            }
            Intent::ToggleMachine { machine } => {
                if let Some(mut m) = machine_at(&mut machines, *machine) {
                    m.state.enabled = !m.state.enabled;
                }
            }
            Intent::Chat { text } => {
                let _ = text;
            }
            Intent::Respawn => {
                if player.dead {
                    player.dead = false;
                    player.vitals = deepcore_shared::Vitals::default();
                    let spawn = world.generator.spawn_point();
                    movement.0 = deepcore_shared::MovementState::at_spawn(spawn);
                    player.inventory_dirty = true;
                }
            }
            Intent::InsertUpgrade { machine, slot } => {
                if let Some(mut m) = machine_at(&mut machines, *machine) {
                    if let Some(stack) = player.inventory.slots.get(usize::from(*slot)).and_then(|s| s.stack) {
                        if matches!(content.item(stack.item).def.category, ItemCategory::Upgrade(_)) {
                            if m.state.upgrades.len() < 2 {
                                m.state.upgrades.push(stack.item);
                                let _ = player.inventory.take_from_slot(usize::from(*slot), 1);
                                player.inventory_dirty = true;
                            } else {
                                out.push("Machine already has two modules".into());
                            }
                        }
                    }
                }
            }
            Intent::RefillTool { boiler } => {
                let mut refilled = false;
                if let Some(m) = machine_at(&mut machines, *boiler) {
                    if m.state.burn_remaining > 0.0 || m.state.fuel.iter().any(|f| f.count() > 0) {
                        refilled = true;
                    }
                }
                if refilled {
                    player.tool_energy = 300.0;
                    player.inventory_dirty = true;
                    out.push("Canister refilled".into());
                } else {
                    out.push("Boiler has no fuel".into());
                }
            }
            Intent::RequestMachine { machine } => {
                if let Some(m) = machine_at(&mut machines, *machine) {
                    contents.write(ToClients {
                        targets: SendTargets::Single(ClientId::Client(client_entity)),
                        message: machine_contents(&content, &m),
                    });
                    player.open_machine = Some(*machine);
                }
            }
        }
        held.0 = player.inventory.selected_stack().map(|s| s.item);
        for text in out {
            notifies.write(ToClients {
                targets: SendTargets::Single(ClientId::Client(client_entity)),
                message: Notify { text, level: 1 },
            });
        }
        let _ = &mut scanners;
    }
}

fn machine_at<'a>(
    machines: &'a mut Query<&mut ServerMachine>,
    pos: BlockPos,
) -> Option<Mut<'a, ServerMachine>> {
    for m in machines.iter_mut() {
        if m.pos == pos {
            return Some(m);
        }
    }
    None
}

/// Validates and performs a block placement.
#[allow(clippy::too_many_arguments)]
fn place_block(
    commands: &mut Commands,
    content: &GameContent,
    world: &mut World,
    params: &MovementParams,
    player: &mut ServerPlayer,
    movement: &NetMovement,
    machines: &mut Query<&mut ServerMachine>,
    queue: &mut BlockChangeQueue,
    cache: &mut ChunkByteCache,
    machine_index: &mut MachineIndex,
    pos: BlockPos,
    face: deepcore_shared::Face,
    slot: u8,
    facing: u8,
    out: &mut Vec<String>,
) {
    let reach = content.tuning.reach + 1.0;
    let eye = movement.0.eye(&params.0);
    if BlockPos::from_world(eye).distance_squared_to(pos.center()) > reach * reach {
        out.push("Too far away".into());
        return;
    }
    if world.block(pos) != BlockId::AIR {
        out.push("Something is already there".into());
        return;
    }
    let Some(stack) = player.inventory.slots.get(usize::from(slot)).and_then(|s| s.stack) else {
        return;
    };
    let item_id = stack.item;
    let item = content.item(stack.item);
    let target_block = match &item.def.category {
        ItemCategory::Block(block) => content.block_id(block),
        ItemCategory::Machine(machine) => content
            .machine_id(machine)
            .and_then(|m| content.machine(m).block),
        _ => None,
    };
    let Some(block) = target_block else {
        out.push("That item cannot be placed".into());
        return;
    };
    // Do not place inside the player.
    let mut player_aabb = movement.0.position;
    player_aabb[1] += 0.0;
    let px = player_aabb[0] - params.0.radius;
    let px2 = player_aabb[0] + params.0.radius;
    let pz = player_aabb[2] - params.0.radius;
    let pz2 = player_aabb[2] + params.0.radius;
    let py = player_aabb[1];
    let py2 = player_aabb[1] + params.0.height;
    if (pos.x as f32) < px2
        && (pos.x as f32 + 1.0) > px
        && (pos.z as f32) < pz2
        && (pos.z as f32 + 1.0) > pz
        && (pos.y as f32) < py2
        && (pos.y as f32 + 1.0) > py
    {
        out.push("You are standing there".into());
        return;
    }
    let _ = face;
    world.set_block(pos, block);
    cache.invalidate(deepcore_shared::voxel::ChunkPos::from_block(pos));
    queue.changes.push(BlockChangeMsg { pos, block });
    let _ = player.inventory.take_from_slot(usize::from(slot), 1);
    player.inventory_dirty = true;

    // Machine placement creates a block entity.
    if let Some(machine_id) = content.block(block).machine {
        let placed = crate::sim::logistics::try_place_machine(
            commands,
            content,
            world,
            machine_index,
            machine_id,
            pos,
            facing,
            queue,
        );
        if placed.is_none() {
            out.push("Not enough space for this machine".into());
            // Undo the block so the player keeps the item.
            world.set_block(pos, BlockId::AIR);
            queue.changes.push(BlockChangeMsg { pos, block: BlockId::AIR });
            let _ = player.inventory.insert(content, item_id, 1);
        }
    }
}

/// Opens a machine UI or uses an interaction target.
fn interact(
    content: &GameContent,
    params: &MovementParams,
    player: &mut ServerPlayer,
    movement: &NetMovement,
    machines: &mut Query<&mut ServerMachine>,
    pos: BlockPos,
    out: &mut Vec<String>,
) {
    let eye = movement.0.eye(&params.0);
    let reach = content.tuning.reach + 1.0;
    if BlockPos::from_world(eye).distance_squared_to(pos.center()) > reach * reach {
        out.push("Too far away".into());
        return;
    }
    for m in machines.iter_mut() {
        if m.pos == pos {
            player.open_machine = Some(pos);
            return;
        }
    }
    let _ = out;
}

/// Hand crafting at a workbench: consumes inputs, waits, produces outputs.
fn craft(content: &GameContent, player: &mut ServerPlayer, recipe: RecipeId, count: u16, out: &mut Vec<String>) {
    let def = content.recipe(recipe);
    if def.machine != content.machine_id("workbench").unwrap_or(deepcore_shared::content::ids::MachineId(0)) {
        out.push("That must be crafted in a machine".into());
        return;
    }
    let mut crafted = 0;
    for _ in 0..count.max(1) {
        let stacks: Vec<(ItemId, u16)> = player
            .inventory
            .slots
            .iter()
            .filter_map(|s| s.stack.map(|st| (st.item, st.count)))
            .collect();
        let Some(found) = find_craftable(content, def.machine, &stacks, |_| true) else {
            break;
        };
        let recipe_def = content.recipe(found);
        for (item, needed) in &recipe_def.inputs {
            player.inventory.remove(*item, *needed);
        }
        for (item, produced) in &recipe_def.outputs {
            let result = player.inventory.insert(content, *item, *produced);
            if result.leftover > 0 {
                out.push("Inventory full".into());
            }
        }
        crafted += 1;
        if found != recipe {
            break;
        }
    }
    if crafted == 0 {
        out.push("Missing materials".into());
    }
    player.inventory_dirty = true;
}

/// Moves items between the player and a machine.
#[allow(clippy::too_many_arguments)]
fn machine_transfer(
    content: &GameContent,
    player: &mut ServerPlayer,
    machines: &mut Query<&mut ServerMachine>,
    pos: BlockPos,
    from_player: Option<u16>,
    from_machine: Option<u8>,
    to_machine: Option<u8>,
    to_player: Option<u16>,
    count: u16,
    out: &mut Vec<String>,
) {
    let Some(mut machine) = machine_at(machines, pos) else {
        out.push("No machine there".into());
        return;
    };
    let moved = if let Some(player_slot) = from_player {
        // Player → machine.
        crate::sim::logistics::insert_into_machine(
            content,
            &mut player.inventory,
            &mut machine,
            player_slot,
            count,
        )
    } else if let Some(machine_slot) = from_machine {
        // Machine → player.
        crate::sim::logistics::take_from_machine(
            content,
            &mut machine,
            &mut player.inventory,
            machine_slot,
            count,
        )
    } else {
        0
    };
    let _ = (to_machine, to_player);
    if moved == 0 {
        out.push("Nothing moved".into());
    }
    player.inventory_dirty = true;
}

/// Sets the recipe a machine runs, if researched.
fn set_machine_recipe(
    content: &GameContent,
    research: &Research,
    machines: &mut Query<&mut ServerMachine>,
    pos: BlockPos,
    recipe: Option<RecipeId>,
    out: &mut Vec<String>,
) {
    let Some(mut machine) = machine_at(machines, pos) else {
        out.push("No machine there".into());
        return;
    };
    match recipe {
        None => machine.state.recipe = None,
        Some(id) => {
            let def = content.recipe(id);
            if def.machine != machine.machine {
                out.push("That recipe does not fit this machine".into());
                return;
            }
            if !research.unlocked(content, def.tech) {
                out.push("Requires research".into());
                return;
            }
            machine.state.recipe = Some(id);
            machine.state.progress = 0.0;
        }
    }
}

/// Delivers items from the inventory to a tech node.
fn deposit_research(
    content: &GameContent,
    player: &mut ServerPlayer,
    research: &mut Research,
    node: TechId,
    out: &mut Vec<String>,
) {
    if research.is_researched(node) {
        out.push("Already researched".into());
        return;
    }
    let tech = content.tech(node);
    // All parents must be done first.
    if !tech.parents.iter().all(|p| research.is_researched(*p)) {
        out.push("Parent research is incomplete".into());
        return;
    }
    let mut delivered = research.delivered(node);
    for (item, needed) in &tech.cost {
        let already: u32 = delivered
            .iter()
            .filter(|(i, _)| i == item)
            .map(|(_, c)| u32::from(*c))
            .sum();
        let missing = u32::from(*needed).saturating_sub(already);
        if missing == 0 {
            continue;
        }
        let take = player.inventory.count_of(*item).min(missing) as u16;
        if take == 0 {
            continue;
        }
        player.inventory.remove(*item, take);
        if let Some(entry) = delivered.iter_mut().find(|(i, _)| i == item) {
            entry.1 += take;
        } else {
            delivered.push((*item, take));
        }
    }
    // Complete the node if everything arrived.
    let complete = tech.cost.iter().all(|(item, needed)| {
        delivered
            .iter()
            .filter(|(i, _)| i == item)
            .map(|(_, c)| u32::from(*c))
            .sum::<u32>()
            >= u32::from(*needed)
    });
    research.progress.retain(|(n, _)| *n != node);
    if complete {
        research.researched.push(node);
        info!("research completed: {}", tech.def.name);
    } else {
        research.progress.push((node, delivered));
    }
    player.inventory_dirty = true;
}

/// Builds the message describing a machine's full contents.
pub fn machine_contents(content: &GameContent, machine: &ServerMachine) -> MachineContents {
    let slot = |s: &Slot| s.stack;
    MachineContents {
        pos: machine.pos,
        machine: machine.machine,
        inputs: machine.state.inputs.iter().map(slot).collect(),
        outputs: machine.state.outputs.iter().map(slot).collect(),
        fuel: machine.state.fuel.iter().map(slot).collect(),
        recipe: machine.state.recipe,
        progress: machine.state.display_progress,
        idle: machine.state.idle.message().to_string(),
        upgrades: machine.state.upgrades.clone(),
        enabled: machine.state.enabled,
    }
}

/// Sends machine contents to players who have that machine open.
pub fn send_machine_contents(
    content: Res<GameContent>,
    machines: Query<&ServerMachine>,
    players: Query<&ServerPlayer>,
    sessions: Query<(Entity, &crate::net::handshake::ClientSession)>,
    mut messages: MessageWriter<ToClients<MachineContents>>,
) {
    for (client_entity, session) in sessions.iter() {
        let Some(player_entity) = session.player else {
            continue;
        };
        let Ok(player) = players.get(player_entity) else {
            continue;
        };
        let Some(pos) = player.open_machine else {
            continue;
        };
        let Some(machine) = machines.iter().find(|m| m.pos == pos) else {
            continue;
        };
        messages.write(ToClients {
            targets: SendTargets::Single(ClientId::Client(client_entity)),
            message: machine_contents(&content, machine),
        });
    }
}

/// Sends an inventory update to one client.
pub fn push_inventory(
    player: &ServerPlayer,
    target: SendTargets,
    messages: &mut MessageWriter<ToClients<InventoryUpdate>>,
) {
    messages.write(ToClients {
        targets: target,
        message: InventoryUpdate {
            inventory: player.inventory.clone(),
            vitals: player.vitals,
        },
    });
}

/// Unused helper kept for the ore scanner intent.
#[allow(dead_code)]
pub fn scan_for_ore(
    content: &GameContent,
    world: &World,
    centre: BlockPos,
    radius: i32,
) -> Vec<(BlockPos, BlockId)> {
    let mut found = Vec::new();
    for dy in -radius..=radius {
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let pos = centre.offset(dx, dy, dz);
                let block = world.block(pos);
                if block == BlockId::AIR {
                    continue;
                }
                if matches!(
                    content.block(block).def.pattern,
                    deepcore_shared::content::defs::Pattern::OreFleck(_)
                ) {
                    found.push((pos, block));
                }
            }
        }
    }
    found
}

/// Runs one tick of a processor machine (used by tests and the machine system).
pub fn run_machine_tick(
    content: &GameContent,
    machine: &mut ServerMachine,
    dt: f32,
    power_factor: f32,
    research: &Research,
) -> TickOutcome {
    let role = content.machine(machine.machine).def.role;
    if role != MachineRole::Processor {
        return TickOutcome::Idle(deepcore_shared::IdleReason::NoRecipe);
    }
    tick_processor(content, &mut machine.state, dt, power_factor, |gate| {
        research.unlocked(content, gate)
    })
}

/// Marks the player's inventory as changed.
pub fn dirty(player: &mut ServerPlayer) {
    player.inventory_dirty = true;
}

/// Returns a clone of the machine state at a position (used by the reactor).
pub fn machine_state_at(machines: &Query<&ServerMachine>, pos: BlockPos) -> Option<MachineState> {
    machines
        .iter()
        .find(|m| m.pos == pos)
        .map(|m| m.state.clone())
}

/// The machine query type used across the sim modules.
pub type MachineQuery<'w, 's> = Query<'w, 's, &'static mut ServerMachine>;

/// A block position pair used by the network overlay.
pub type NetPos = (BlockPos, f32);
