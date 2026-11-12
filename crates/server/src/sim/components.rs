//! Server-side player and machine components plus their bookkeeping resources.

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::content::ids::{ItemId, MachineId, RecipeId, TechId};
use deepcore_shared::crafting::MachineState;
use deepcore_shared::hazards::{Protection, Vitals};
use deepcore_shared::inventory::Inventory;
use deepcore_shared::movement::{MoveInput, MovementState};
use deepcore_shared::protocol::{NetHeld, NetHealth, NetMachine, NetMovement, NetPlayer};
use deepcore_shared::voxel::BlockPos;

/// A connected, spawned player on the server.
#[derive(Component)]
pub struct ServerPlayer {
    /// Owning client.
    pub client: ClientId,
    /// Display name.
    pub name: String,
    /// Item storage.
    pub inventory: Inventory,
    /// Vitals.
    pub vitals: Vitals,
    /// Worn protection gear.
    pub protection: Protection,
    /// Currently selected tool, if any.
    pub tool: Option<ItemId>,
    /// Remaining tool energy.
    pub tool_energy: f32,
    /// Mining state.
    pub mining: Option<MiningJob>,
    /// Pending movement inputs in sequence order.
    pub pending_inputs: Vec<MoveInput>,
    /// Last sequence number processed.
    pub last_seq: u32,
    /// Whether the inventory changed since it was last sent.
    pub inventory_dirty: bool,
    /// Whether the player is dead and waiting to respawn.
    pub dead: bool,
    /// Machine whose UI this player has open.
    pub open_machine: Option<BlockPos>,
    /// Last tick a scanner sweep was run.
    pub last_scan: u32,
}

/// A block the player is currently mining.
#[derive(Debug, Clone, Copy)]
pub struct MiningJob {
    /// Targeted block.
    pub block: BlockPos,
    /// Targeted face.
    pub face: deepcore_shared::voxel::Face,
    /// Seconds of progress.
    pub progress: f32,
    /// Seconds needed in total.
    pub total: f32,
    /// Tool tier used.
    pub tier: u8,
}

impl ServerPlayer {
    /// Builds the server-side state for a new player.
    pub fn new(client: ClientId, name: String, spawn: [f32; 3]) -> Self {
        let mut inventory = Inventory::new();
        // Every player starts with a steel pickaxe in the first hotbar slot.
        inventory.slots[0] = deepcore_shared::inventory::Slot::with(ItemId(0), 1);
        Self {
            client,
            name,
            inventory,
            vitals: Vitals::default(),
            protection: Protection::default(),
            tool: None,
            tool_energy: 0.0,
            mining: None,
            pending_inputs: Vec::new(),
            last_seq: 0,
            inventory_dirty: true,
            dead: false,
            open_machine: None,
            last_scan: 0,
        }
    }

    /// The replicated movement state.
    pub fn movement(&self) -> Option<&MovementState> {
        None
    }
}

/// A machine placed in the world.
#[derive(Component)]
pub struct ServerMachine {
    /// Machine type.
    pub machine: MachineId,
    /// Origin block position.
    pub pos: BlockPos,
    /// Facing 0..3.
    pub facing: u8,
    /// Simulation state.
    pub state: MachineState,
    /// Position in the electric or steam network, if attached to one.
    pub network_pos: Option<BlockPos>,
    /// Reactor core temperature (reactor only).
    pub core_temp: f32,
    /// Reactor control rod insertion 0..1 (reactor only).
    pub rod_insertion: f32,
    /// Reactor output in kW (reactor only).
    pub reactor_kw: f32,
    /// Whether the reactor has been scrammed.
    pub scrameed: bool,
}

/// The team's research state.
#[derive(Resource, Default)]
pub struct Research {
    /// Researched nodes.
    pub researched: Vec<TechId>,
    /// Items delivered towards each unresearched node.
    pub progress: Vec<(TechId, Vec<(ItemId, u16)>)>,
}

impl Research {
    /// Whether a node is researched.
    pub fn is_researched(&self, node: TechId) -> bool {
        self.researched.contains(&node)
    }

    /// Whether a tech gate is satisfied. `None` gates are always open.
    pub fn unlocked(&self, content: &deepcore_shared::Content, gate: Option<TechId>) -> bool {
        match gate {
            None => true,
            Some(node) => {
                let _ = content;
                self.is_researched(node)
            }
        }
    }

    /// Delivered items for a node.
    pub fn delivered(&self, node: TechId) -> Vec<(ItemId, u16)> {
        self.progress
            .iter()
            .find(|(n, _)| *n == node)
            .map(|(_, items)| items.clone())
            .unwrap_or_default()
    }
}

/// Index of all placed machines by position, rebuilt on placement/removal.
#[derive(Resource, Default)]
pub struct MachineIndex {
    /// Machine entity at each occupied block position.
    pub by_pos: std::collections::HashMap<BlockPos, Entity>,
}

/// Queue of block changes to broadcast this tick.
#[derive(Resource, Default)]
pub struct BlockChangeQueue {
    /// Pending changes.
    pub changes: Vec<deepcore_shared::protocol::BlockChangeMsg>,
}

/// Dropped item pickups waiting to be spawned.
#[derive(Resource, Default)]
pub struct PickupQueue {
    /// Items to drop.
    pub items: Vec<(ItemId, u16, [f32; 3])>,
}

/// The server's simulation clock.
#[derive(Resource, Default)]
pub struct SimClock {
    /// Tick counter.
    pub tick: u32,
}

/// Replicated marker helpers so the client and server agree on component shapes.
pub fn player_bundle(name: String, state: MovementState) -> impl Bundle {
    (
        Replicated,
        NetPlayer { name, alive: true },
        NetMovement(state),
        NetHealth(100.0),
        NetHeld(None),
        Transform::from_xyz(state.position[0], state.position[1], state.position[2]),
    )
}

/// Replicated machine bundle.
pub fn machine_bundle(machine: MachineId, pos: BlockPos, facing: u8) -> impl Bundle {
    (
        Replicated,
        NetMachine { machine, pos, progress: 0.0, running: false, facing },
        Transform::from_xyz(pos.x as f32, pos.y as f32, pos.z as f32),
    )
}

/// Convenience accessor for the recipe currently set on a machine.
pub fn machine_recipe(machine: &ServerMachine) -> Option<RecipeId> {
    machine.state.recipe
}
