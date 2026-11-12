//! Data definitions for all Deepcore content.
//!
//! These structs mirror the RON files in `assets/data/`. They are the *authoring* format: at load
//! time every string id is resolved into a compact numeric id by [`crate::content::registry`].

use serde::{Deserialize, Serialize};

/// A linear RGB colour written as `(r, g, b)` in RON.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgb(pub f32, pub f32, pub f32);

impl Rgb {
    /// Returns the colour as an array.
    pub fn to_array(self) -> [f32; 3] {
        [self.0, self.1, self.2]
    }
}

/// Procedural texture pattern used by the atlas generator.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Pattern {
    /// Flat colour.
    Solid,
    /// Fine noise speckle, for stone-like blocks.
    Speckle,
    /// Bright flecks of the given colour inside the base colour, for ore blocks.
    OreFleck(Rgb),
    /// Horizontal banding, for sedimentary layers.
    Bands,
    /// Diagonal cracks, for bedrock-like blocks.
    Cracks,
    /// Soft glow in the centre, for emissive blocks.
    Glow,
}

/// Radiation emitted by a block placed in the world.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RadiationDef {
    /// Radius in blocks that the block irradiates.
    pub radius: f32,
    /// Dose per second at the centre of the radius.
    pub dose_per_sec: f32,
}

fn default_true() -> bool {
    true
}

/// A block type.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockDef {
    /// Unique string id.
    pub id: String,
    /// Seconds to break with a tier-1 tool.
    pub hardness: f32,
    /// Lowest tool tier that can break this block.
    pub min_tier: u8,
    /// Items dropped when mined, as `(item_id, count)`.
    #[serde(default)]
    pub drops: Vec<(String, u16)>,
    /// Base colour of the procedural texture.
    pub color: Rgb,
    /// Procedural texture pattern.
    pub pattern: Pattern,
    /// Emissive strength, 0 = not emissive.
    #[serde(default)]
    pub emissive: f32,
    /// Radiation emitted while placed.
    #[serde(default)]
    pub radiation: Option<RadiationDef>,
    /// Whether the block blocks movement and vision.
    #[serde(default = "default_true")]
    pub solid: bool,
    /// Whether the block is unbreakable (bedrock).
    #[serde(default)]
    pub unbreakable: bool,
    /// Which layer this block belongs to, for documentation and the atlas.
    #[serde(default)]
    pub layer_hint: Option<u8>,
    /// If set, placing this block spawns the machine with this id.
    #[serde(default)]
    pub machine: Option<String>,
    /// Whether the block is a liquid (water) that flows.
    #[serde(default)]
    pub liquid: bool,
}

/// Tool statistics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    /// Mining tier; must be at least a block's `min_tier` to break it.
    pub tier: u8,
    /// Speed multiplier in the break-time formula.
    pub speed: f32,
    /// Mining area on the targeted face, as `(width, height)` in blocks.
    pub area: (u8, u8),
    /// How the tool is powered.
    pub energy: ToolEnergy,
}

/// How a tool is powered.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ToolEnergy {
    /// No energy needed.
    Manual,
    /// Steam canisters, refilled at a boiler.
    Steam {
        /// Canister capacity in steam units.
        capacity: f32,
    },
    /// Battery packs, recharged at a charging station.
    Battery {
        /// Battery capacity in energy units.
        capacity: f32,
    },
    /// A nuclear core cell, good for a fixed number of breaks.
    CoreCell {
        /// Number of block breaks one core cell provides.
        breaks: u32,
    },
}

/// Upgrade module effect.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UpgradeDef {
    /// Speed multiplier added to the machine speed.
    pub speed: f32,
    /// Power multiplier added to the machine power use.
    pub power: f32,
}

/// Item category and any behaviour attached to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ItemCategory {
    /// A plain crafting material.
    Material,
    /// A tool the player can hold.
    Tool(ToolDef),
    /// Places a block when used.
    Block(String),
    /// Places a machine when used.
    Machine(String),
    /// An upgrade module for machines.
    Upgrade(UpgradeDef),
    /// A consumable such as a canister refill.
    Consumable,
    /// A fuel item with a burn duration in seconds.
    Fuel {
        /// Seconds the item burns for.
        burn_secs: f32,
    },
}

/// An item type.
#[derive(Debug, Clone, Deserialize)]
pub struct ItemDef {
    /// Unique string id.
    pub id: String,
    /// Player-facing name.
    pub name: String,
    /// Maximum stack size.
    pub stack: u16,
    /// Category and behaviour.
    pub category: ItemCategory,
}

/// How a machine gets its energy.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EnergyKind {
    /// Runs by hand.
    None,
    /// Burns fuel items.
    Fuel {
        /// Number of fuel slots.
        slots: u8,
    },
    /// Draws steam units per second from a pipe network.
    Steam {
        /// Steam units drawn per second.
        rate: f32,
    },
    /// Draws kilowatts from a cable network.
    Electric {
        /// Kilowatts drawn.
        kw: f32,
    },
}

impl EnergyKind {
    /// Steam drawn per second, or zero.
    pub fn steam_rate(&self) -> f32 {
        match self {
            EnergyKind::Steam { rate } => *rate,
            _ => 0.0,
        }
    }

    /// Electric kilowatts drawn, or zero.
    pub fn electric_kw(&self) -> f32 {
        match self {
            EnergyKind::Electric { kw } => *kw,
            _ => 0.0,
        }
    }

    /// Number of fuel slots.
    pub fn fuel_slots(&self) -> u8 {
        match self {
            EnergyKind::Fuel { slots } => *slots,
            _ => 0,
        }
    }
}

/// What a machine does beyond processing recipes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MachineRole {
    /// Runs recipes.
    Processor,
    /// Burns fuel to add steam to a pipe network.
    Boiler {
        /// Steam units produced per second.
        steam_per_sec: f32,
    },
    /// Consumes steam to generate electricity.
    SteamEngine {
        /// Steam units consumed per second.
        steam_per_sec: f32,
        /// Kilowatts produced.
        kw: f32,
    },
    /// Burns fuel to generate electricity.
    Generator {
        /// Kilowatts produced while burning fuel.
        kw: f32,
    },
    /// Stores electricity.
    Battery {
        /// Storage capacity in kilowatt hours.
        capacity_kwh: f32,
    },
    /// Opens the tech tree UI.
    ResearchDesk,
    /// Recharges battery tools.
    ChargingStation,
    /// The reactor controller.
    Reactor,
    /// A decorative light.
    Lamp,
    /// Item storage with slots but no processing.
    Storage,
    /// A utility structure (fan, pump, shower, console, lift) with no recipe processing.
    Utility,
}

/// Centrifuge cascade constraints.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CascadeDef {
    /// Minimum number of connected centrifuges for the cascade to run.
    pub min: u8,
    /// Maximum number of connected centrifuges that contribute.
    pub max: u8,
}

/// A machine type.
#[derive(Debug, Clone, Deserialize)]
pub struct MachineDef {
    /// Unique string id.
    pub id: String,
    /// Player-facing name.
    pub name: String,
    /// Energy input.
    pub energy: EnergyKind,
    /// Number of input slots.
    pub input_slots: u8,
    /// Number of output slots.
    pub output_slots: u8,
    /// Items needed to build it, as `(item_id, count)`.
    pub build_cost: Vec<(String, u16)>,
    /// Tech node that unlocks it.
    #[serde(default)]
    pub tech: Option<String>,
    /// Footprint in blocks, as `(width, height, depth)`.
    #[serde(default = "one")]
    pub size: (u8, u8, u8),
    /// Cascade rules for centrifuges.
    #[serde(default)]
    pub cascade: Option<CascadeDef>,
    /// Extra behaviour.
    pub role: MachineRole,
    /// Block id used to render the machine in the world.
    #[serde(default)]
    pub block: Option<String>,
}

fn one() -> (u8, u8, u8) {
    (1, 1, 1)
}

/// A processing recipe.
#[derive(Debug, Clone, Deserialize)]
pub struct RecipeDef {
    /// Unique string id.
    pub id: String,
    /// Machine that runs it.
    pub machine: String,
    /// Duration in seconds at full speed.
    pub time: f32,
    /// Inputs consumed, as `(item_id, count)`.
    pub inputs: Vec<(String, u16)>,
    /// Outputs produced, as `(item_id, count)`.
    pub outputs: Vec<(String, u16)>,
    /// Tech node required before the recipe can be selected.
    #[serde(default)]
    pub tech: Option<String>,
}

/// A tech tree node.
#[derive(Debug, Clone, Deserialize)]
pub struct TechDef {
    /// Unique string id.
    pub id: String,
    /// Player-facing name.
    pub name: String,
    /// Era this node belongs to.
    pub era: u8,
    /// Parent nodes that must be researched first.
    #[serde(default)]
    pub parents: Vec<String>,
    /// Items that must be delivered to the research desk.
    pub cost: Vec<(String, u16)>,
    /// Names of machines, blocks or recipes unlocked by this node.
    #[serde(default)]
    pub unlocks: Vec<String>,
    /// One-line description shown in the tech tree UI.
    #[serde(default)]
    pub description: String,
}

/// Ore placement rule inside a layer.
#[derive(Debug, Clone, Deserialize)]
pub struct OreRule {
    /// Ore block id.
    pub block: String,
    /// Vein spawn probability per noise cell, 0..1.
    pub frequency: f32,
    /// Vein size range in blocks.
    pub vein_size: (u32, u32),
}

/// A hazard type that can appear in a layer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum HazardKind {
    /// Water pockets that flood when broken.
    Flooding {
        /// Chance per chunk to place a pocket.
        pocket_chance: f32,
        /// Maximum pocket volume in blocks.
        max_volume: u32,
    },
    /// Methane pockets that drain oxygen and explode near flames.
    Gas {
        /// Chance per chunk to place a pocket.
        pocket_chance: f32,
    },
    /// Ambient heat that raises body heat.
    Heat {
        /// Extra degrees per second at the layer's base depth.
        rise: f32,
    },
    /// Unsupported ceilings can collapse.
    CaveIn {
        /// Maximum supported span in blocks.
        max_span: u8,
    },
    /// Ambient radiation.
    Radiation {
        /// Dose per second in the layer.
        dose_per_sec: f32,
    },
    /// Complete darkness.
    Darkness,
}

/// One of the six depth layers.
#[derive(Debug, Clone, Deserialize)]
pub struct LayerDef {
    /// Unique string id.
    pub id: String,
    /// Player-facing name.
    pub name: String,
    /// Depth range below the surface, as `(top, bottom)`.
    pub depth: (u32, u32),
    /// Block that fills the layer by default.
    pub base_block: String,
    /// Secondary fill blocks with relative weights.
    #[serde(default)]
    pub filler: Vec<(String, f32)>,
    /// Ore placement rules.
    #[serde(default)]
    pub ores: Vec<OreRule>,
    /// Hazards present in the layer.
    #[serde(default)]
    pub hazards: Vec<HazardKind>,
    /// Ambient temperature in degrees Celsius.
    #[serde(default)]
    pub ambient_temp: f32,
}

/// Tunable gameplay constants that are not tied to a single piece of content.
#[derive(Debug, Clone, Deserialize)]
pub struct Tuning {
    /// World width and depth in blocks.
    pub world_size: (u32, u32),
    /// World height in blocks.
    pub world_height: u32,
    /// Y coordinate of the surface.
    pub surface_y: u32,
    /// Gravity in blocks per second squared.
    pub gravity: f32,
    /// Walk speed in blocks per second.
    pub walk_speed: f32,
    /// Sprint speed in blocks per second.
    pub sprint_speed: f32,
    /// Crouch speed in blocks per second.
    pub crouch_speed: f32,
    /// Jump velocity in blocks per second.
    pub jump_speed: f32,
    /// Player half-width.
    pub player_radius: f32,
    /// Player height.
    pub player_height: f32,
    /// Eye height above the feet.
    pub eye_height: f32,
    /// Reach distance in blocks.
    pub reach: f32,
    /// Reach distance with a powered drill.
    pub drill_reach: f32,
    /// Seconds of fuel burn per coal unit, used by the balance report.
    pub coal_burn_secs: f32,
    /// Oxygen drain per second underwater.
    pub oxygen_drain: f32,
    /// Oxygen refill per second in air.
    pub oxygen_refill: f32,
    /// Radiation dose that starts screen grain.
    pub dose_grain: f32,
    /// Radiation dose that stops health regeneration.
    pub dose_no_regen: f32,
    /// Radiation dose that deals damage.
    pub dose_damage: f32,
    /// Radiation damage per second at `dose_damage`.
    pub radiation_dps: f32,
    /// Dose removed per second away from sources.
    pub dose_decay: f32,
    /// Body heat comfortable maximum.
    pub heat_limit: f32,
    /// Body heat damage per second above the limit.
    pub heat_dps: f32,
    /// Server tick rate in Hz.
    pub tick_hz: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            world_size: (1024, 1024),
            world_height: 512,
            surface_y: 480,
            gravity: 28.0,
            walk_speed: 4.5,
            sprint_speed: 7.0,
            crouch_speed: 2.0,
            jump_speed: 8.4,
            player_radius: 0.3,
            player_height: 1.8,
            eye_height: 1.62,
            reach: 5.0,
            drill_reach: 6.0,
            coal_burn_secs: 80.0,
            oxygen_drain: 6.0,
            oxygen_refill: 20.0,
            dose_grain: 100.0,
            dose_no_regen: 300.0,
            dose_damage: 600.0,
            radiation_dps: 2.0,
            dose_decay: 0.6,
            heat_limit: 45.0,
            heat_dps: 1.5,
            tick_hz: 30.0,
        }
    }
}
