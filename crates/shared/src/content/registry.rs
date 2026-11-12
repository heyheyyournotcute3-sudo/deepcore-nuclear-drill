//! Loading, validation and lookup for all game content.

use std::collections::HashMap;
use std::path::Path;

use serde::de::DeserializeOwned;

use super::defs::*;
use super::ids::*;

/// Anything that can go wrong while loading content.
#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    /// A content file could not be read.
    #[error("failed to read {path}: {source}")]
    Io {
        /// File that failed.
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },
    /// A content file was not valid RON.
    #[error("failed to parse {path}: {source}")]
    Parse {
        /// File that failed.
        path: String,
        /// Underlying error.
        source: ron::error::SpannedError,
    },
    /// The content is syntactically fine but semantically broken.
    #[error("{0}")]
    Invalid(String),
}

impl ContentError {
    fn invalid(msg: impl Into<String>) -> Self {
        ContentError::Invalid(msg.into())
    }
}

/// A block with its references resolved.
#[derive(Debug, Clone)]
pub struct Block {
    /// The authored definition.
    pub def: BlockDef,
    /// Resolved drop table.
    pub drops: Vec<(ItemId, u16)>,
    /// Machine spawned by placing this block.
    pub machine: Option<MachineId>,
}

/// An item with its references resolved.
#[derive(Debug, Clone)]
pub struct Item {
    /// The authored definition.
    pub def: ItemDef,
    /// Block placed by this item.
    pub block: Option<BlockId>,
    /// Machine placed by this item.
    pub machine: Option<MachineId>,
}

/// A recipe with its references resolved.
#[derive(Debug, Clone)]
pub struct Recipe {
    /// The authored definition.
    pub def: RecipeDef,
    /// Machine that runs the recipe.
    pub machine: MachineId,
    /// Resolved inputs.
    pub inputs: Vec<(ItemId, u16)>,
    /// Resolved outputs.
    pub outputs: Vec<(ItemId, u16)>,
    /// Tech node that unlocks it.
    pub tech: Option<TechId>,
}

/// A machine with its references resolved.
#[derive(Debug, Clone)]
pub struct Machine {
    /// The authored definition.
    pub def: MachineDef,
    /// Resolved build cost.
    pub build_cost: Vec<(ItemId, u16)>,
    /// Tech node that unlocks it.
    pub tech: Option<TechId>,
    /// Block that represents it in the world.
    pub block: Option<BlockId>,
    /// Volume of the footprint in blocks.
    pub volume: u32,
}

/// A tech node with its references resolved.
#[derive(Debug, Clone)]
pub struct Tech {
    /// The authored definition.
    pub def: TechDef,
    /// Resolved parents.
    pub parents: Vec<TechId>,
    /// Resolved research cost.
    pub cost: Vec<(ItemId, u16)>,
}

/// Every piece of content in the game, indexed by numeric id.
#[derive(Debug, Clone)]
pub struct Content {
    /// Blocks, indexed by [`BlockId`]. Index 0 is air.
    pub blocks: Vec<Block>,
    /// Items, indexed by [`ItemId`].
    pub items: Vec<Item>,
    /// Recipes, indexed by [`RecipeId`].
    pub recipes: Vec<Recipe>,
    /// Machines, indexed by [`MachineId`].
    pub machines: Vec<Machine>,
    /// Tech nodes, indexed by [`TechId`].
    pub techs: Vec<Tech>,
    /// The six depth layers, top to bottom.
    pub layers: Vec<LayerDef>,
    /// Tunable constants.
    pub tuning: Tuning,
    /// Stable hash of every content file, used for the handshake.
    pub hash: u64,
    block_index: HashMap<String, BlockId>,
    item_index: HashMap<String, ItemId>,
    recipe_index: HashMap<String, RecipeId>,
    machine_index: HashMap<String, MachineId>,
    tech_index: HashMap<String, TechId>,
}

/// The RON files that make up the content set.
pub const CONTENT_FILES: [&str; 8] = [
    "blocks.ron",
    "items.ron",
    "recipes.ron",
    "machines.ron",
    "tech_tree.ron",
    "layers.ron",
    "hazards.ron",
    "tuning.ron",
];

fn read_file(dir: &Path, name: &str) -> Result<(String, Vec<u8>), ContentError> {
    let path = dir.join(name);
    let bytes = std::fs::read(&path).map_err(|source| ContentError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let text = String::from_utf8(bytes.clone()).map_err(|source| ContentError::Io {
        path: path.display().to_string(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
    })?;
    Ok((text, bytes))
}

fn parse<T: DeserializeOwned>(dir: &Path, name: &str) -> Result<T, ContentError> {
    let (text, _) = read_file(dir, name)?;
    ron::from_str(&text).map_err(|source| ContentError::Parse {
        path: dir.join(name).display().to_string(),
        source,
    })
}

/// FNV-1a over the raw bytes of every content file, in file-name order.
fn hash_files(dir: &Path) -> Result<u64, ContentError> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for name in CONTENT_FILES {
        let (_, bytes) = read_file(dir, name)?;
        for b in name.as_bytes().iter().chain(bytes.iter()) {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
    }
    Ok(hash)
}

impl Content {
    /// Loads and validates every content file from `dir`.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Content, ContentError> {
        let dir = dir.as_ref();
        let block_defs: Vec<BlockDef> = parse(dir, "blocks.ron")?;
        let item_defs: Vec<ItemDef> = parse(dir, "items.ron")?;
        let recipe_defs: Vec<RecipeDef> = parse(dir, "recipes.ron")?;
        let machine_defs: Vec<MachineDef> = parse(dir, "machines.ron")?;
        let tech_defs: Vec<TechDef> = parse(dir, "tech_tree.ron")?;
        let layers: Vec<LayerDef> = parse(dir, "layers.ron")?;
        // The hazards file holds layer-level hazard tuning overrides; it is parsed for validation
        // even though layer hazards themselves live in `layers.ron`.
        let hazard_overrides: Vec<HazardOverride> = parse(dir, "hazards.ron")?;
        let tuning: Tuning = parse(dir, "tuning.ron")?;
        let hash = hash_files(dir)?;
        Content::build(
            block_defs,
            item_defs,
            recipe_defs,
            machine_defs,
            tech_defs,
            layers,
            hazard_overrides,
            tuning,
            hash,
        )
    }

    /// Builds a validated [`Content`] from already-parsed definitions.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        block_defs: Vec<BlockDef>,
        item_defs: Vec<ItemDef>,
        recipe_defs: Vec<RecipeDef>,
        machine_defs: Vec<MachineDef>,
        tech_defs: Vec<TechDef>,
        layers: Vec<LayerDef>,
        hazard_overrides: Vec<HazardOverride>,
        tuning: Tuning,
        hash: u64,
    ) -> Result<Content, ContentError> {
        if block_defs.iter().any(|b| b.id == "air") {
            return Err(ContentError::invalid("blocks.ron must not define 'air'"));
        }
        let mut block_index = HashMap::new();
        block_index.insert("air".to_string(), BlockId::AIR);
        for (i, def) in block_defs.iter().enumerate() {
            let id = BlockId(u16::try_from(i + 1).map_err(|_| {
                ContentError::invalid("too many blocks: BlockId is a u16 and 0 is air")
            })?);
            if block_index.insert(def.id.clone(), id).is_some() {
                return Err(ContentError::invalid(format!("duplicate block id '{}'", def.id)));
            }
        }

        let mut item_index = HashMap::new();
        for (i, def) in item_defs.iter().enumerate() {
            let id = ItemId(
                u16::try_from(i).map_err(|_| ContentError::invalid("too many items for u16 ids"))?,
            );
            if item_index.insert(def.id.clone(), id).is_some() {
                return Err(ContentError::invalid(format!("duplicate item id '{}'", def.id)));
            }
        }

        let mut machine_index = HashMap::new();
        for (i, def) in machine_defs.iter().enumerate() {
            let id = MachineId(
                u16::try_from(i)
                    .map_err(|_| ContentError::invalid("too many machines for u16 ids"))?,
            );
            if machine_index.insert(def.id.clone(), id).is_some() {
                return Err(ContentError::invalid(format!(
                    "duplicate machine id '{}'",
                    def.id
                )));
            }
        }

        let mut tech_index = HashMap::new();
        for (i, def) in tech_defs.iter().enumerate() {
            let id = TechId(
                u16::try_from(i).map_err(|_| ContentError::invalid("too many tech nodes"))?,
            );
            if tech_index.insert(def.id.clone(), id).is_some() {
                return Err(ContentError::invalid(format!(
                    "duplicate tech id '{}'",
                    def.id
                )));
            }
        }

        let mut recipe_index = HashMap::new();
        for (i, def) in recipe_defs.iter().enumerate() {
            let id = RecipeId(
                u16::try_from(i).map_err(|_| ContentError::invalid("too many recipes"))?,
            );
            if recipe_index.insert(def.id.clone(), id).is_some() {
                return Err(ContentError::invalid(format!(
                    "duplicate recipe id '{}'",
                    def.id
                )));
            }
        }

        let item_id = |name: &str| -> Result<ItemId, ContentError> {
            item_index
                .get(name)
                .copied()
                .ok_or_else(|| ContentError::invalid(format!("unknown item id '{name}'")))
        };
        let block_id = |name: &str| -> Result<BlockId, ContentError> {
            block_index
                .get(name)
                .copied()
                .ok_or_else(|| ContentError::invalid(format!("unknown block id '{name}'")))
        };
        let machine_id = |name: &str| -> Result<MachineId, ContentError> {
            machine_index
                .get(name)
                .copied()
                .ok_or_else(|| ContentError::invalid(format!("unknown machine id '{name}'")))
        };
        let tech_id = |name: &str| -> Result<TechId, ContentError> {
            tech_index
                .get(name)
                .copied()
                .ok_or_else(|| ContentError::invalid(format!("unknown tech id '{name}'")))
        };

        // --- blocks -------------------------------------------------------
        let mut blocks = Vec::with_capacity(block_defs.len() + 1);
        blocks.push(Block {
            def: BlockDef {
                id: "air".into(),
                hardness: 0.0,
                min_tier: 0,
                drops: Vec::new(),
                color: Rgb(0.0, 0.0, 0.0),
                pattern: Pattern::Solid,
                emissive: 0.0,
                radiation: None,
                solid: false,
                unbreakable: false,
                layer_hint: None,
                machine: None,
                liquid: false,
            },
            drops: Vec::new(),
            machine: None,
        });
        for def in block_defs {
            let mut drops = Vec::new();
            for (item, count) in &def.drops {
                drops.push((item_id(item)?, *count));
            }
            let machine = def.machine.as_deref().map(machine_id).transpose()?;
            blocks.push(Block { def, drops, machine });
        }

        // --- items --------------------------------------------------------
        let mut items = Vec::with_capacity(item_defs.len());
        for def in item_defs {
            let mut block = None;
            let mut machine = None;
            match &def.category {
                ItemCategory::Block(b) => block = Some(block_id(b)?),
                ItemCategory::Machine(m) => machine = Some(machine_id(m)?),
                _ => {}
            }
            items.push(Item { def, block, machine });
        }

        // --- machines -----------------------------------------------------
        let mut machines = Vec::with_capacity(machine_defs.len());
        for def in machine_defs {
            let mut build_cost = Vec::new();
            for (item, count) in &def.build_cost {
                build_cost.push((item_id(item)?, *count));
            }
            let tech = def.tech.as_deref().map(tech_id).transpose()?;
            let block = def.block.as_deref().map(block_id).transpose()?;
            let volume = u32::from(def.size.0) * u32::from(def.size.1) * u32::from(def.size.2);
            machines.push(Machine { def, build_cost, tech, block, volume });
        }

        // --- tech ---------------------------------------------------------
        let mut techs = Vec::with_capacity(tech_defs.len());
        for def in tech_defs {
            let mut parents = Vec::new();
            for p in &def.parents {
                parents.push(tech_id(p)?);
            }
            let mut cost = Vec::new();
            for (item, count) in &def.cost {
                cost.push((item_id(item)?, *count));
            }
            techs.push(Tech { def, parents, cost });
        }

        // --- recipes ------------------------------------------------------
        let mut recipes = Vec::with_capacity(recipe_defs.len());
        for def in recipe_defs {
            let machine = machine_id(&def.machine)?;
            let mut inputs = Vec::new();
            for (item, count) in &def.inputs {
                inputs.push((item_id(item)?, *count));
            }
            let mut outputs = Vec::new();
            for (item, count) in &def.outputs {
                outputs.push((item_id(item)?, *count));
            }
            let tech = def.tech.as_deref().map(tech_id).transpose()?;
            recipes.push(Recipe { def, machine, inputs, outputs, tech });
        }

        let content = Content {
            blocks,
            items,
            recipes,
            machines,
            techs,
            layers,
            tuning,
            hash,
            block_index,
            item_index,
            recipe_index,
            machine_index,
            tech_index,
        };

        let mut problems = content.validate();
        problems.extend(validate_hazard_overrides(&content, &hazard_overrides));
        if !problems.is_empty() {
            let joined: Vec<String> = problems.iter().take(20).cloned().collect();
            return Err(ContentError::invalid(format!(
                "content validation failed ({} problems):\n  {}",
                problems.len(),
                joined.join("\n  ")
            )));
        }
        Ok(content)
    }

    fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();

        // Every recipe must name a machine that can run it, and every machine that is a
        // processor must have at least one recipe.
        for recipe in &self.recipes {
            let machine = &self.machines[recipe.machine.0 as usize];
            if machine.def.role != MachineRole::Processor {
                problems.push(format!(
                    "recipe '{}' targets machine '{}' which is not a processor",
                    recipe.def.id, machine.def.id
                ));
            }
            if recipe.inputs.is_empty() {
                problems.push(format!("recipe '{}' has no inputs", recipe.def.id));
            }
            if recipe.outputs.is_empty() {
                problems.push(format!("recipe '{}' has no outputs", recipe.def.id));
            }
            if recipe.def.time <= 0.0 {
                problems.push(format!("recipe '{}' has a non-positive time", recipe.def.id));
            }
        }
        for machine in &self.machines {
            if machine.def.role == MachineRole::Processor
                && machine.def.input_slots > 0
                && !self.recipes.iter().any(|r| r.machine.0 == self.machine_id(&machine.def.id).unwrap_or(MachineId(0)).0)
            {
                problems.push(format!(
                    "machine '{}' is a processor but no recipe uses it",
                    machine.def.id
                ));
            }
            if machine.def.size.0 == 0 || machine.def.size.1 == 0 || machine.def.size.2 == 0 {
                problems.push(format!("machine '{}' has a zero-sized footprint", machine.def.id));
            }
        }

        // Tech nodes: parents are resolved at load time, and the graph must be acyclic.
        if let Some(cycle) = self.find_tech_cycle() {
            problems.push(format!("tech tree contains a cycle through '{}'", cycle));
        }
        for tech in &self.techs {
            for unlock in &tech.def.unlocks {
                if self.machine_index.contains_key(unlock)
                    || self.recipe_index.contains_key(unlock)
                    || self.block_index.contains_key(unlock)
                    || self.item_index.contains_key(unlock)
                {
                    continue;
                }
                problems.push(format!(
                    "tech '{}' unlocks unknown thing '{}'",
                    tech.def.id, unlock
                ));
            }
        }

        // Every machine gated by a tech node must be listed in that node's unlocks.
        for machine in &self.machines {
            if let Some(tech) = machine.tech {
                let node = &self.techs[tech.0 as usize];
                if !node.def.unlocks.iter().any(|u| u == &machine.def.id) {
                    problems.push(format!(
                        "machine '{}' is gated by tech '{}' but not listed in its unlocks",
                        machine.def.id, node.def.id
                    ));
                }
            }
        }

        // Ores must have a use: the item they drop must appear in some recipe, build cost or
        // research cost.
        for block in &self.blocks {
            if block.drops.is_empty() {
                continue;
            }
            let is_ore = matches!(block.def.pattern, Pattern::OreFleck(_))
                || block.def.radiation.is_some();
            for (item, _) in &block.drops {
                if is_ore && !self.item_has_use(*item) {
                    problems.push(format!(
                        "ore block '{}' drops '{}' which has no use in any recipe, build cost or research cost",
                        block.def.id, self.items[item.0 as usize].def.id
                    ));
                }
            }
        }

        // Layers must reference real blocks and ores, cover 0..world_height contiguously and
        // be ordered top to bottom.
        let mut expected_top = 0u32;
        for layer in &self.layers {
            if layer.depth.0 != expected_top {
                problems.push(format!(
                    "layer '{}' starts at {} but the previous layer ended at {}",
                    layer.id, layer.depth.0, expected_top
                ));
            }
            if layer.depth.1 <= layer.depth.0 {
                problems.push(format!("layer '{}' has an empty depth range", layer.id));
            }
            expected_top = layer.depth.1;
            if !self.block_index.contains_key(&layer.base_block) {
                problems.push(format!(
                    "layer '{}' uses unknown base block '{}'",
                    layer.id, layer.base_block
                ));
            }
            for (filler, weight) in &layer.filler {
                if !self.block_index.contains_key(filler) {
                    problems.push(format!(
                        "layer '{}' uses unknown filler block '{}'",
                        layer.id, filler
                    ));
                }
                if *weight <= 0.0 {
                    problems.push(format!(
                        "layer '{}' filler '{}' has a non-positive weight",
                        layer.id, filler
                    ));
                }
            }
            for ore in &layer.ores {
                if !self.block_index.contains_key(&ore.block) {
                    problems.push(format!(
                        "layer '{}' references unknown ore block '{}'",
                        layer.id, ore.block
                    ));
                }
                if ore.vein_size.0 == 0 || ore.vein_size.1 < ore.vein_size.0 {
                    problems.push(format!(
                        "layer '{}' ore '{}' has an invalid vein size range",
                        layer.id, ore.block
                    ));
                }
            }
        }

        // Every tool item must have a matching tier path: at least one tool of each tier 1..=5.
        for tier in 1..=5u8 {
            let found = self.items.iter().any(|i| {
                matches!(&i.def.category, ItemCategory::Tool(t) if t.tier == tier)
            });
            if !found {
                problems.push(format!("no tool of tier {tier} exists"));
            }
        }

        // A reactor, nuclear drill and every nuclear-chain machine must exist.
        for required in [
            "reactor",
            "nuclear_drill",
            "leaching_tank",
            "precipitator",
            "conversion_plant",
            "centrifuge",
            "fuel_fabricator",
        ] {
            if !self.machine_index.contains_key(required) && !self.item_index.contains_key(required)
            {
                problems.push(format!("required content '{required}' is missing"));
            }
        }

        problems
    }

    fn find_tech_cycle(&self) -> Option<&str> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            New,
            Open,
            Done,
        }
        fn visit(
            content: &Content,
            id: TechId,
            marks: &mut [Mark],
        ) -> Option<TechId> {
            match marks[id.0 as usize] {
                Mark::Done => return None,
                Mark::Open => return Some(id),
                Mark::New => {}
            }
            marks[id.0 as usize] = Mark::Open;
            for parent in &content.techs[id.0 as usize].parents {
                if let Some(cycle) = visit(content, *parent, marks) {
                    return Some(cycle);
                }
            }
            marks[id.0 as usize] = Mark::Done;
            None
        }
        let mut marks = vec![Mark::New; self.techs.len()];
        for i in 0..self.techs.len() {
            if let Some(cycle) = visit(self, TechId(i as u16), &mut marks) {
                return Some(&self.techs[cycle.0 as usize].def.id);
            }
        }
        None
    }

    /// Whether an item is consumed or produced by anything at all.
    pub fn item_has_use(&self, item: ItemId) -> bool {
        let in_recipe = self
            .recipes
            .iter()
            .any(|r| r.inputs.iter().chain(r.outputs.iter()).any(|(i, _)| *i == item));
        let in_build = self
            .machines
            .iter()
            .any(|m| m.build_cost.iter().any(|(i, _)| *i == item));
        let in_research = self.techs.iter().any(|t| t.cost.iter().any(|(i, _)| *i == item));
        in_recipe || in_build || in_research
    }

    /// Looks up a block by string id.
    pub fn block_id(&self, name: &str) -> Option<BlockId> {
        self.block_index.get(name).copied()
    }

    /// Looks up an item by string id.
    pub fn item_id(&self, name: &str) -> Option<ItemId> {
        self.item_index.get(name).copied()
    }

    /// Looks up a recipe by string id.
    pub fn recipe_id(&self, name: &str) -> Option<RecipeId> {
        self.recipe_index.get(name).copied()
    }

    /// Looks up a machine by string id.
    pub fn machine_id(&self, name: &str) -> Option<MachineId> {
        self.machine_index.get(name).copied()
    }

    /// Looks up a tech node by string id.
    pub fn tech_id(&self, name: &str) -> Option<TechId> {
        self.tech_index.get(name).copied()
    }

    /// Block definition.
    pub fn block(&self, id: BlockId) -> &Block {
        &self.blocks[id.0 as usize]
    }

    /// Item definition.
    pub fn item(&self, id: ItemId) -> &Item {
        &self.items[id.0 as usize]
    }

    /// Recipe definition.
    pub fn recipe(&self, id: RecipeId) -> &Recipe {
        &self.recipes[id.0 as usize]
    }

    /// Machine definition.
    pub fn machine(&self, id: MachineId) -> &Machine {
        &self.machines[id.0 as usize]
    }

    /// Tech node.
    pub fn tech(&self, id: TechId) -> &Tech {
        &self.techs[id.0 as usize]
    }

    /// Player-facing name of an item.
    pub fn item_name(&self, id: ItemId) -> &str {
        &self.items[id.0 as usize].def.name
    }

    /// Recipes that a machine can run, in registry order.
    pub fn recipes_for(&self, machine: MachineId) -> impl Iterator<Item = RecipeId> + '_ {
        self.recipes
            .iter()
            .enumerate()
            .filter(move |(_, r)| r.machine == machine)
            .map(|(i, _)| RecipeId(i as u16))
    }

    /// The layer containing a world Y coordinate (0 = deepest).
    pub fn layer_at_y(&self, y: u32) -> Option<&LayerDef> {
        let depth = self.tuning.surface_y.saturating_sub(y);
        self.layers
            .iter()
            .find(|l| depth >= l.depth.0 && depth < l.depth.1)
    }

    /// Index of the layer containing a world Y coordinate.
    pub fn layer_index_at_y(&self, y: u32) -> Option<usize> {
        let depth = self.tuning.surface_y.saturating_sub(y);
        self.layers
            .iter()
            .position(|l| depth >= l.depth.0 && depth < l.depth.1)
    }
}

/// A per-layer hazard tweak authored in `hazards.ron`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct HazardOverride {
    /// Layer id the override applies to.
    pub layer: String,
    /// Hazard name, e.g. `"flooding"`.
    pub hazard: String,
    /// Multiplier applied to the layer's hazard parameters.
    #[serde(default = "one_f32")]
    pub multiplier: f32,
}

fn one_f32() -> f32 {
    1.0
}

fn validate_hazard_overrides(content: &Content, overrides: &[HazardOverride]) -> Vec<String> {
    let mut problems = Vec::new();
    for o in overrides {
        if !content.layers.iter().any(|l| l.id == o.layer) {
            problems.push(format!("hazards.ron references unknown layer '{}'", o.layer));
        }
        if !["flooding", "gas", "heat", "cave_in", "radiation", "darkness"]
            .contains(&o.hazard.as_str())
        {
            problems.push(format!("hazards.ron references unknown hazard '{}'", o.hazard));
        }
        if o.multiplier <= 0.0 {
            problems.push(format!(
                "hazards.ron override for '{}' has a non-positive multiplier",
                o.layer
            ));
        }
    }
    problems
}
