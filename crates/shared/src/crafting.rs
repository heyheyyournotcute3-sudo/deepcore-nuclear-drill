//! Recipe matching and the machine processing state machine.

use serde::{Deserialize, Serialize};

use crate::content::Content;
use crate::content::ids::{ItemId, MachineId, RecipeId};
use crate::inventory::{ItemStack, Slot};

/// Why a machine is not running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdleReason {
    /// Nothing to do: no recipe selected or no inputs.
    NoRecipe,
    /// Inputs are missing.
    MissingInputs,
    /// The output slots are full.
    OutputFull,
    /// The machine has no energy.
    NoPower,
    /// The recipe needs research that is not done yet.
    Locked,
    /// A cascade is too short to run.
    CascadeTooShort,
    /// The machine is disabled by its own UI.
    Disabled,
}

impl IdleReason {
    /// Player-facing text for the machine UI.
    pub fn message(self) -> &'static str {
        match self {
            IdleReason::NoRecipe => "No recipe selected",
            IdleReason::MissingInputs => "Missing inputs",
            IdleReason::OutputFull => "Output full",
            IdleReason::NoPower => "No power",
            IdleReason::Locked => "Requires research",
            IdleReason::CascadeTooShort => "Cascade too short",
            IdleReason::Disabled => "Disabled",
        }
    }
}

/// Persistent state of one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MachineState {
    /// Machine type.
    pub machine: MachineId,
    /// Input slots.
    pub inputs: Vec<Slot>,
    /// Output slots.
    pub outputs: Vec<Slot>,
    /// Fuel slots.
    pub fuel: Vec<Slot>,
    /// Selected recipe, if any.
    pub recipe: Option<RecipeId>,
    /// Progress in seconds towards the current recipe.
    pub progress: f32,
    /// Remaining burn time in seconds for the current fuel item.
    pub burn_remaining: f32,
    /// Number of upgrade modules installed.
    pub upgrades: Vec<ItemId>,
    /// Whether the machine is enabled by its owner.
    pub enabled: bool,
    /// Why the machine idled on the last tick, for the UI.
    pub idle: IdleReason,
    /// Coarse progress 0..=1, replicated for the world view.
    pub display_progress: f32,
    /// Whether the machine is currently doing something, replicated for animations.
    pub running: bool,
}

impl MachineState {
    /// Creates a state sized for a machine type.
    pub fn new(content: &Content, machine: MachineId) -> Self {
        let def = &content.machine(machine).def;
        Self {
            machine,
            inputs: vec![Slot::empty(); usize::from(def.input_slots)],
            outputs: vec![Slot::empty(); usize::from(def.output_slots)],
            fuel: vec![Slot::empty(); usize::from(def.energy.fuel_slots())],
            recipe: None,
            progress: 0.0,
            burn_remaining: 0.0,
            upgrades: Vec::new(),
            enabled: true,
            idle: IdleReason::NoRecipe,
            display_progress: 0.0,
            running: false,
        }
    }

    /// Speed multiplier from installed modules.
    pub fn speed_multiplier(&self, content: &Content) -> f32 {
        let mut speed = 1.0;
        for item in &self.upgrades {
            if let crate::content::defs::ItemCategory::Upgrade(u) = &content.item(*item).def.category {
                speed += u.speed;
            }
        }
        speed
    }

    /// Power multiplier from installed modules.
    pub fn power_multiplier(&self, content: &Content) -> f32 {
        let mut power = 1.0;
        for item in &self.upgrades {
            if let crate::content::defs::ItemCategory::Upgrade(u) = &content.item(*item).def.category {
                power *= u.power;
            }
        }
        power.max(0.1)
    }

    /// Whether the output slots can accept the outputs of `recipe`.
    pub fn can_output(&self, content: &Content, recipe: RecipeId) -> bool {
        let def = content.recipe(recipe);
        let mut simulated: Vec<Slot> = self.outputs.clone();
        for (item, count) in &def.outputs {
            let max = Inventory::max_stack(content, *item);
            let mut remaining = *count;
            for slot in &mut simulated {
                if remaining == 0 {
                    break;
                }
                match &mut slot.stack {
                    None => {
                        let take = max.min(remaining);
                        slot.stack = Some(ItemStack::new(*item, take));
                        remaining -= take;
                    }
                    Some(stack) if stack.item == *item && stack.count < max => {
                        let take = (max - stack.count).min(remaining);
                        stack.count += take;
                        remaining -= take;
                    }
                    _ => {}
                }
            }
            if remaining > 0 {
                return false;
            }
        }
        true
    }

    /// Whether the input slots hold everything `recipe` needs.
    pub fn has_inputs(&self, content: &Content, recipe: RecipeId) -> bool {
        let def = content.recipe(recipe);
        for (item, count) in &def.inputs {
            let available: u32 = self
                .inputs
                .iter()
                .filter_map(|s| s.stack)
                .filter(|s| s.item == *item)
                .map(|s| u32::from(s.count))
                .sum();
            if available < u32::from(*count) {
                return false;
            }
        }
        true
    }

    /// Consumes the inputs of `recipe`.
    pub fn consume_inputs(&mut self, content: &Content, recipe: RecipeId) {
        let def = content.recipe(recipe);
        for (item, count) in &def.inputs {
            let mut remaining = *count;
            for slot in &mut self.inputs {
                if remaining == 0 {
                    break;
                }
                if let Some(stack) = &mut slot.stack {
                    if stack.item == *item {
                        let take = stack.count.min(remaining);
                        stack.count -= take;
                        remaining -= take;
                        if stack.count == 0 {
                            slot.stack = None;
                        }
                    }
                }
            }
        }
    }

    /// Adds the outputs of `recipe` to the output slots.
    pub fn produce_outputs(&mut self, content: &Content, recipe: RecipeId) {
        let def = content.recipe(recipe);
        for (item, count) in &def.outputs {
            let max = Inventory::max_stack(content, *item);
            let mut remaining = *count;
            for slot in &mut self.outputs {
                if remaining == 0 {
                    break;
                }
                match &mut slot.stack {
                    None => {
                        let take = max.min(remaining);
                        slot.stack = Some(ItemStack::new(*item, take));
                        remaining -= take;
                    }
                    Some(stack) if stack.item == *item && stack.count < max => {
                        let take = (max - stack.count).min(remaining);
                        stack.count += take;
                        remaining -= take;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Total fuel burn time available, including the item currently burning.
    pub fn fuel_available(&self, content: &Content) -> f32 {
        let mut total = self.burn_remaining;
        for slot in &self.fuel {
            if let Some(stack) = slot.stack {
                if let crate::content::defs::ItemCategory::Fuel { burn_secs } =
                    &content.item(stack.item).def.category
                {
                    total += burn_secs * f32::from(stack.count);
                }
            }
        }
        total
    }

    /// Burns one fuel item if nothing is burning.
    pub fn refuel(&mut self, content: &Content) -> bool {
        if self.burn_remaining > 0.0 {
            return true;
        }
        for slot in &mut self.fuel {
            if let Some(stack) = &mut slot.stack {
                if let crate::content::defs::ItemCategory::Fuel { burn_secs } =
                    &content.item(stack.item).def.category
                {
                    self.burn_remaining = *burn_secs;
                    stack.count -= 1;
                    if stack.count == 0 {
                        slot.stack = None;
                    }
                    return true;
                }
            }
        }
        false
    }

    /// Advances the burn timer by `dt` seconds.
    pub fn tick_fuel(&mut self, content: &Content, dt: f32) {
        if self.burn_remaining > 0.0 {
            self.burn_remaining = (self.burn_remaining - dt).max(0.0);
        }
        let _ = content;
    }
}

use crate::inventory::Inventory;

/// The result of ticking a machine once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// The machine completed a recipe this tick.
    Completed(RecipeId),
    /// The machine is making progress.
    Working,
    /// The machine is idle for the given reason.
    Idle(IdleReason),
}

/// Ticks a processor machine.
///
/// `power_factor` is the fraction of the required energy that is available (1.0 = full power,
/// less means a brownout). `fuel_factor` reports whether a fuel-burning machine has fuel.
pub fn tick_processor(
    content: &Content,
    state: &mut MachineState,
    dt: f32,
    power_factor: f32,
    unlocked: impl Fn(Option<crate::content::ids::TechId>) -> bool,
) -> TickOutcome {
    if !state.enabled {
        state.running = false;
        state.idle = IdleReason::Disabled;
        return TickOutcome::Idle(IdleReason::Disabled);
    }

    let Some(recipe) = state.recipe else {
        state.running = false;
        state.idle = IdleReason::NoRecipe;
        return TickOutcome::Idle(IdleReason::NoRecipe);
    };
    let def = content.recipe(recipe);
    if !unlocked(def.tech) {
        state.running = false;
        state.idle = IdleReason::Locked;
        return TickOutcome::Idle(IdleReason::Locked);
    }

    // Fuel machines burn fuel only while they have work to do.
    let needs_fuel = matches!(content.machine(state.machine).def.energy, crate::content::defs::EnergyKind::Fuel { .. });
    if needs_fuel {
        let has_work = state.has_inputs(content, recipe) && state.can_output(content, recipe);
        if has_work && !state.refuel(content) {
            state.running = false;
            state.idle = IdleReason::NoPower;
            return TickOutcome::Idle(IdleReason::NoPower);
        }
    }
    if power_factor <= 0.0 {
        state.running = false;
        state.idle = IdleReason::NoPower;
        return TickOutcome::Idle(IdleReason::NoPower);
    }

    if !state.has_inputs(content, recipe) {
        state.running = false;
        state.idle = IdleReason::MissingInputs;
        return TickOutcome::Idle(IdleReason::MissingInputs);
    }
    if !state.can_output(content, recipe) {
        state.running = false;
        state.idle = IdleReason::OutputFull;
        return TickOutcome::Idle(IdleReason::OutputFull);
    }

    state.running = true;
    state.idle = IdleReason::NoRecipe;
    let speed = state.speed_multiplier(content);
    state.progress += dt * speed * power_factor.min(1.0);
    if needs_fuel {
        state.tick_fuel(content, dt * power_factor.min(1.0));
    }
    state.display_progress = (state.progress / def.def.time.max(0.01)).clamp(0.0, 1.0);

    if state.progress >= def.def.time {
        state.progress = 0.0;
        state.display_progress = 0.0;
        state.consume_inputs(content, recipe);
        state.produce_outputs(content, recipe);
        TickOutcome::Completed(recipe)
    } else {
        TickOutcome::Working
    }
}

/// Finds the first recipe of a machine whose inputs are satisfied by a set of stacks.
/// Used by hand crafting at the workbench.
pub fn find_craftable<'a>(
    content: &'a Content,
    machine: MachineId,
    stacks: &[(ItemId, u16)],
    unlocked: impl Fn(Option<crate::content::ids::TechId>) -> bool + 'a,
) -> Option<RecipeId> {
    content
        .recipes_for(machine)
        .find(|recipe| {
            let def = content.recipe(*recipe);
            if !unlocked(def.tech) {
                return false;
            }
            def.inputs.iter().all(|(item, count)| {
                let available: u32 = stacks
                    .iter()
                    .filter(|(i, _)| i == item)
                    .map(|(_, c)| u32::from(*c))
                    .sum();
                available >= u32::from(*count)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> Content {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/data");
        Content::load_from_dir(dir).expect("content loads")
    }

    #[test]
    fn furnace_smelts_when_fed() {
        let c = content();
        let furnace = c.machine_id("stone_furnace").expect("furnace");
        let raw_iron = c.item_id("raw_iron").expect("raw_iron");
        let coal = c.item_id("coal").expect("coal");
        let mut state = MachineState::new(&c, furnace);
        state.inputs[0] = Slot::with(raw_iron, 1);
        state.fuel[0] = Slot::with(coal, 1);
        let recipe = c.recipe_id("smelt_iron").expect("recipe");
        state.recipe = Some(recipe);
        let mut completed = false;
        for _ in 0..1000 {
            if let TickOutcome::Completed(_) = tick_processor(&c, &mut state, 0.1, 1.0, |_| true) {
                completed = true;
                break;
            }
        }
        assert!(completed, "furnace should finish smelting");
        let iron = c.item_id("iron_ingot").expect("iron");
        assert_eq!(state.outputs.iter().map(|s| s.count()).sum::<u16>(), 1);
        assert_eq!(state.outputs[0].item(), Some(iron));
    }

    #[test]
    fn no_power_when_no_fuel() {
        let c = content();
        let furnace = c.machine_id("stone_furnace").expect("furnace");
        let mut state = MachineState::new(&c, furnace);
        state.recipe = Some(c.recipe_id("smelt_iron").expect("recipe"));
        state.inputs[0] = Slot::with(c.item_id("raw_iron").expect("iron"), 1);
        let outcome = tick_processor(&c, &mut state, 0.1, 1.0, |_| true);
        assert_eq!(outcome, TickOutcome::Idle(IdleReason::NoPower));
    }

    #[test]
    fn missing_inputs_reported() {
        let c = content();
        let furnace = c.machine_id("stone_furnace").expect("furnace");
        let mut state = MachineState::new(&c, furnace);
        state.recipe = Some(c.recipe_id("smelt_iron").expect("recipe"));
        state.fuel[0] = Slot::with(c.item_id("coal").expect("coal"), 1);
        let outcome = tick_processor(&c, &mut state, 0.1, 1.0, |_| true);
        assert_eq!(outcome, TickOutcome::Idle(IdleReason::MissingInputs));
    }

    #[test]
    fn brownout_slows_processing() {
        let c = content();
        let furnace = c.machine_id("stone_furnace").expect("furnace");
        let recipe = c.recipe_id("smelt_iron").expect("recipe");
        let mut full = MachineState::new(&c, furnace);
        let mut half = MachineState::new(&c, furnace);
        for state in [&mut full, &mut half] {
            state.recipe = Some(recipe);
            state.inputs[0] = Slot::with(c.item_id("raw_iron").expect("iron"), 1);
            state.fuel[0] = Slot::with(c.item_id("coal").expect("coal"), 1);
        }
        for _ in 0..10 {
            tick_processor(&c, &mut full, 0.1, 1.0, |_| true);
            tick_processor(&c, &mut half, 0.1, 0.5, |_| true);
        }
        assert!(full.progress > half.progress * 1.5);
    }
}
