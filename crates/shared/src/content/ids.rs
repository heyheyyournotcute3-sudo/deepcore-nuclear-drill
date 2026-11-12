//! Compact numeric ids resolved from the string ids used in the RON files.

use serde::{Deserialize, Serialize};

/// Index of a block type in the block registry. `BlockId(0)` is always air.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BlockId(pub u16);

impl BlockId {
    /// The air block.
    pub const AIR: BlockId = BlockId(0);
}

/// Index of an item type in the item registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ItemId(pub u16);

/// Index of a recipe in the recipe registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RecipeId(pub u16);

/// Index of a machine type in the machine registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MachineId(pub u16);

/// Index of a tech node in the tech registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TechId(pub u16);
