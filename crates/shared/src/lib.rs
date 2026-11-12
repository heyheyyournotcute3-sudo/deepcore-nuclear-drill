//! Deepcore shared game logic.
//!
//! Every rule of the game lives here: content, voxels, world generation, movement, inventory,
//! crafting, power, hazards and the network protocol. The server decides using this code; the
//! client only predicts movement and displays results.

#![deny(missing_docs)]

pub mod content;
pub mod crafting;
pub mod hazards;
pub mod inventory;
pub mod movement;
pub mod power;
pub mod protocol;
pub mod voxel;
pub mod worldgen;

use bevy::prelude::*;

pub use content::{Content, ContentError};
pub use crafting::{IdleReason, MachineState, TickOutcome};
pub use hazards::{Environment, Protection, Vitals};
pub use inventory::{Inventory, ItemStack, Slot};
pub use movement::{MoveInput, MoveParams, MovementState};
pub use power::NetworkIndex;
pub use protocol::PROTOCOL_VERSION;
pub use voxel::{BlockPos, Chunk, ChunkMap, ChunkPos, Face};

/// The content registry, inserted as a resource once it has been loaded.
#[derive(Resource, Debug, Clone)]
pub struct GameContent(pub std::sync::Arc<Content>);

impl std::ops::Deref for GameContent {
    type Target = Content;
    fn deref(&self) -> &Content {
        &self.0
    }
}

/// Movement parameters derived from content, inserted as a resource.
#[derive(Resource, Debug, Clone, Copy)]
pub struct MovementParams(pub MoveParams);

/// The content directory used when none is given on the command line.
pub const DEFAULT_CONTENT_DIR: &str = "assets/data";

/// Loads content and turns it into the resources the game needs.
pub fn load_content_resources(dir: &str) -> Result<(GameContent, MovementParams), ContentError> {
    let content = Content::load_from_dir(dir)?;
    let mut params = MoveParams::default();
    let t = &content.tuning;
    params.gravity = t.gravity;
    params.walk_speed = t.walk_speed;
    params.sprint_speed = t.sprint_speed;
    params.crouch_speed = t.crouch_speed;
    params.jump_speed = t.jump_speed;
    params.radius = t.player_radius;
    params.height = t.player_height;
    params.eye_height = t.eye_height;
    params.dt = 1.0 / t.tick_hz;
    Ok((GameContent(std::sync::Arc::new(content)), MovementParams(params)))
}

/// Registers every replicated component and network message.
///
/// Must be added *after* `RepliconPlugins`, which creates the `ProtocolHasher` resource that
/// registration writes into. Both binaries add it exactly once, so the client and the server can
/// never drift apart.
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        protocol::register_protocol(app);
    }
}

/// Registers the resources and shared systems both sides need.
pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<protocol::Intent>()
            .add_message::<protocol::ChunkData>()
            .add_message::<protocol::ChunkUnload>()
            .add_message::<protocol::BlockChanges>()
            .add_message::<protocol::MovementAck>()
            .add_message::<protocol::InventoryUpdate>()
            .add_message::<protocol::MachineContents>()
            .add_message::<protocol::TechState>()
            .add_message::<protocol::NetworkSnapshot>()
            .add_message::<protocol::ChatLine>()
            .add_message::<protocol::Notify>()
            .add_message::<protocol::HandshakeRejected>()
            .add_message::<protocol::ScannerResult>()
            .add_message::<protocol::ServerInfo>()
            .add_message::<protocol::ClientHello>();
    }
}
