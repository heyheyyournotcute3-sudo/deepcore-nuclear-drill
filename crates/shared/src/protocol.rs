//! Networking shapes: replicated components, client intents and server messages.
//!
//! Bump [`PROTOCOL_VERSION`] whenever a type in this module changes shape.

use bevy::prelude::*;
use bevy_replicon::prelude::{Channel, ClientMessageAppExt, ServerMessageAppExt};
use serde::{Deserialize, Serialize};

use crate::content::ids::{BlockId, ItemId, MachineId, RecipeId, TechId};
use crate::hazards::Vitals;
use crate::inventory::{ItemStack, Inventory};
use crate::movement::{MoveInput, MovementState};
use crate::voxel::chunk::{BlockPos, ChunkPos, Face};

/// Version of the wire protocol. Clients with a different version are refused.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum players on one server.
pub const MAX_PLAYERS: usize = 16;

/// Horizontal view radius in chunks.
pub const VIEW_RADIUS_XZ: i32 = 6;
/// Vertical view radius in chunks.
pub const VIEW_RADIUS_Y: i32 = 3;

// ---------------------------------------------------------------------------
// Replicated components
// ---------------------------------------------------------------------------

/// Marks a replicated player entity on both sides.
#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct NetPlayer {
    /// Display name.
    pub name: String,
    /// Whether the player is connected and spawned.
    pub alive: bool,
}

/// Replicated movement state of a player.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NetMovement(pub MovementState);

/// Replicated public vitals (health only, for teammates' health bars).
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NetHealth(pub f32);

/// The item a player is holding, for other clients' view models.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NetHeld(pub Option<ItemId>);

/// Marks a replicated machine block entity.
#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct NetMachine {
    /// Machine type.
    pub machine: MachineId,
    /// Block position of the machine's origin.
    pub pos: BlockPos,
    /// Coarse progress 0..=1 for the world view.
    pub progress: f32,
    /// Whether the machine is running.
    pub running: bool,
    /// Which way the machine faces (0..3, 90° steps around Y).
    pub facing: u8,
}

/// Marks a replicated dropped item pickup.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NetPickup {
    /// The item stack.
    pub stack: ItemStack,
    /// World position.
    pub pos: [f32; 3],
}

/// Marks a replicated placed lamp or light-emitting block entity.
#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NetLight {
    /// Light position.
    pub pos: BlockPos,
    /// Light radius in blocks.
    pub radius: f32,
    /// Light intensity 0..=1.
    pub intensity: f32,
}

/// Registers every replicated component and every network message, in a fixed order that must be
/// identical on the client and the server.
///
/// This is the single place the protocol is described; both binaries call it exactly once, from
/// `SharedPlugin`.
pub fn register_protocol(app: &mut bevy::prelude::App) {
    use bevy_replicon::prelude::AppRuleExt;

    // Replication rules first: their order is part of replicon's protocol hash.
    app.replicate::<NetPlayer>()
        .replicate::<NetMovement>()
        .replicate::<NetHealth>()
        .replicate::<NetHeld>()
        .replicate::<NetMachine>()
        .replicate::<NetPickup>()
        .replicate::<NetLight>();

    // Client -> server.
    app.add_client_message::<ClientHello>(Channel::Ordered)
        .add_client_message::<Intent>(Channel::Ordered);

    // Server -> client. The order of these calls assigns channel IDs, so it must match exactly.
    app.add_server_message::<ServerInfo>(Channel::Ordered)
        .add_server_message::<HandshakeRejected>(Channel::Ordered)
        .add_server_message::<ChunkData>(Channel::Ordered)
        .add_server_message::<ChunkUnload>(Channel::Ordered)
        .add_server_message::<BlockChanges>(Channel::Ordered)
        .add_server_message::<InventoryUpdate>(Channel::Ordered)
        .add_server_message::<MachineContents>(Channel::Ordered)
        .add_server_message::<TechState>(Channel::Ordered)
        .add_server_message::<ChatLine>(Channel::Ordered)
        .add_server_message::<Notify>(Channel::Ordered)
        .add_server_message::<ScannerResult>(Channel::Ordered)
        .add_server_message::<MovementAck>(Channel::Unordered)
        .add_server_message::<NetworkSnapshot>(Channel::Unordered);
}

// ---------------------------------------------------------------------------
// Client → server intents
// ---------------------------------------------------------------------------

/// An intent sent from a client to the server.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub enum Intent {
    /// Movement input for one fixed tick.
    Move(MoveInput),
    /// Begin mining the block under the crosshair.
    StartMining {
        /// Targeted block.
        block: BlockPos,
        /// Targeted face.
        face: Face,
    },
    /// Stop mining.
    StopMining,
    /// Place the held item.
    PlaceBlock {
        /// Position to place into.
        pos: BlockPos,
        /// Face that was clicked.
        face: Face,
        /// Hotbar slot used.
        slot: u8,
        /// Facing for machines, 0..3.
        facing: u8,
    },
    /// Interact with a block or machine.
    Interact {
        /// Targeted block.
        pos: BlockPos,
    },
    /// Move items between two inventory slots.
    MoveItems {
        /// Source slot.
        from: u16,
        /// Destination slot.
        to: u16,
        /// How many items to move.
        count: u16,
    },
    /// Quick-move a slot between hotbar and backpack.
    QuickMove {
        /// Slot to move.
        slot: u16,
    },
    /// Drop items from a slot into the world.
    DropItem {
        /// Slot to drop from.
        slot: u16,
        /// How many items to drop.
        count: u16,
    },
    /// Select a hotbar slot.
    SelectSlot {
        /// Slot index 0..8.
        slot: u8,
    },
    /// Craft at the workbench by hand.
    Craft {
        /// Recipe to craft.
        recipe: RecipeId,
        /// How many times.
        count: u16,
    },
    /// Move items between the player's inventory and a machine.
    MachineTransfer {
        /// Machine position.
        machine: BlockPos,
        /// Inventory slot, or `None` for a machine slot.
        from_player: Option<u16>,
        /// Machine slot index, or `None` for a player slot.
        from_machine: Option<u8>,
        /// Destination machine slot.
        to_machine: Option<u8>,
        /// Destination player slot.
        to_player: Option<u16>,
        /// How many items.
        count: u16,
    },
    /// Set the recipe a machine runs.
    SetMachineRecipe {
        /// Machine position.
        machine: BlockPos,
        /// Recipe to run, or `None` to clear.
        recipe: Option<RecipeId>,
    },
    /// Deliver items to a tech node at the research desk.
    ResearchDeposit {
        /// Tech node.
        node: TechId,
    },
    /// Toggle a machine's enabled state.
    ToggleMachine {
        /// Machine position.
        machine: BlockPos,
    },
    /// Send a chat message.
    Chat {
        /// Message text.
        text: String,
    },
    /// Respawn after death.
    Respawn,
    /// Insert an upgrade module into a machine.
    InsertUpgrade {
        /// Machine position.
        machine: BlockPos,
        /// Inventory slot holding the module.
        slot: u16,
    },
    /// Refill a steam drill canister at a boiler.
    RefillTool {
        /// Boiler position.
        boiler: BlockPos,
    },
    /// Request the full slot contents of a machine (sent when its UI opens).
    RequestMachine {
        /// Machine position.
        machine: BlockPos,
    },
}

// ---------------------------------------------------------------------------
// Server → client messages
// ---------------------------------------------------------------------------

/// A chunk's data, streamed to a client.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct ChunkData {
    /// Chunk position.
    pub pos: ChunkPos,
    /// zstd-compressed, postcard-encoded chunk.
    pub bytes: Vec<u8>,
}

/// Chunks the client should unload.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct ChunkUnload {
    /// Chunk positions to drop.
    pub positions: Vec<ChunkPos>,
}

/// A batch of block changes.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct BlockChanges {
    /// Changes applied this tick.
    pub changes: Vec<BlockChangeMsg>,
}

/// One block change.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BlockChangeMsg {
    /// Position.
    pub pos: BlockPos,
    /// New block.
    pub block: BlockId,
}

/// The authoritative movement state for the receiving client, with the last processed input.
#[derive(Message, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MovementAck {
    /// Authoritative state.
    pub state: MovementState,
    /// Last input sequence number the server processed.
    pub last_seq: u32,
    /// Server tick the state belongs to.
    pub tick: u32,
}

/// The owner's inventory contents, sent on change.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct InventoryUpdate {
    /// The full inventory.
    pub inventory: Inventory,
    /// The owner's vitals.
    pub vitals: Vitals,
}

/// The full contents of a machine, sent while its UI is open.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct MachineContents {
    /// Machine position.
    pub pos: BlockPos,
    /// Machine type.
    pub machine: MachineId,
    /// Input slots.
    pub inputs: Vec<Option<ItemStack>>,
    /// Output slots.
    pub outputs: Vec<Option<ItemStack>>,
    /// Fuel slots.
    pub fuel: Vec<Option<ItemStack>>,
    /// Selected recipe.
    pub recipe: Option<RecipeId>,
    /// Progress 0..=1.
    pub progress: f32,
    /// Why the machine is idle.
    pub idle: String,
    /// Installed upgrade modules.
    pub upgrades: Vec<ItemId>,
    /// Whether the machine is enabled.
    pub enabled: bool,
}

/// Team-shared tech tree state.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct TechState {
    /// Researched nodes.
    pub researched: Vec<TechId>,
    /// Items already delivered towards each unresearched node.
    pub progress: Vec<(TechId, Vec<(ItemId, u16)>)>,
}

/// Network overlay statistics.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSnapshot {
    /// Steam and electric networks.
    pub stats: crate::power::NetworkStats,
}

/// A chat line or notification.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct ChatLine {
    /// Sender name, or `None` for a system message.
    pub from: Option<String>,
    /// Message text.
    pub text: String,
}

/// A short-lived notification shown in the HUD ("Too hard — needs Tier 3").
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct Notify {
    /// Message text.
    pub text: String,
    /// Severity 0 = info, 1 = warning, 2 = error.
    pub level: u8,
}

/// Handshake rejection reasons.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeRejected {
    /// Human-readable reason.
    pub reason: String,
}

/// A block position that the client should highlight (used by the Ore Scanner).
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct ScannerResult {
    /// Found ore positions.
    pub positions: Vec<BlockPos>,
    /// Their block ids, parallel to `positions`.
    pub blocks: Vec<BlockId>,
}

/// The server's content hash and protocol version, sent on connect.
#[derive(Message, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Protocol version.
    pub protocol: u32,
    /// Content hash.
    pub content_hash: u64,
    /// World seed.
    pub seed: u64,
    /// Current server tick.
    pub tick: u32,
}

/// Sent by the client right after connecting.
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
pub struct ClientHello {
    /// Protocol version.
    pub protocol: u32,
    /// Content hash.
    pub content_hash: u64,
    /// Display name.
    pub name: String,
    /// Optional lobby auth token (M10).
    pub token: Option<String>,
}
