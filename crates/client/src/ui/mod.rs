//! egui screens: main menu, inventory, machine UI, tech tree, network overlay and HUD.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use bevy_replicon::prelude::*;

use deepcore_shared::content::defs::{ItemCategory, MachineRole};
use deepcore_shared::content::ids::{ItemId, RecipeId, TechId};
use deepcore_shared::inventory::{HOTBAR_SLOTS, PLAYER_SLOTS};
use deepcore_shared::protocol::Intent;
use deepcore_shared::GameContent;

use crate::hud::{self, DebugOverlay, Geiger};
use crate::input::InputState;
use crate::net::{self, ClientGameState, ClientNet};
use crate::prediction::Prediction;
use crate::render::Mesher;
use crate::world::{ClientSettings, ClientWorld};

/// A request from the menu to connect to a server.
#[derive(Resource, Default)]
pub struct ConnectRequest(pub Option<String>);

/// Which panel the player has open.
#[derive(Resource, Default, PartialEq, Eq, Clone, Copy)]
pub enum Panel {
    /// No panel: the game view.
    #[default]
    None,
    /// The main menu.
    Menu,
    /// The inventory screen.
    Inventory,
    /// A machine's UI.
    Machine,
    /// The tech tree.
    TechTree,
}

/// UI plugin state.
#[derive(Resource, Default)]
pub struct UiState {
    /// Open panel.
    pub panel: Panel,
    /// Address typed into the join box.
    pub address: String,
    /// Name typed into the join box.
    pub name: String,
    /// Selected tech node.
    pub selected_tech: Option<TechId>,
    /// Selected recipe for the open machine.
    pub selected_recipe: Option<RecipeId>,
    /// Item held on the mouse cursor while dragging.
    pub drag: Option<(u16, u16)>,
    /// Whether the network overlay is open.
    pub show_network: bool,
}

/// The UI plugin.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiState>()
            .init_resource::<crate::hud::Geiger>()
            .init_resource::<ConnectRequest>()
            .add_systems(
                Update,
                (panel_keys, sync_focus, crate::hud::sync_camera_far, process_connect_request),
            )
            .add_systems(EguiPrimaryContextPass, draw_ui);
    }
}

fn panel_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui: ResMut<UiState>,
    state: Res<State<ClientState>>,
    mut net: ResMut<ClientNet>,
    settings: Res<ClientSettings>,
) {
    if keys.just_pressed(KeyCode::KeyE) {
        ui.panel = match ui.panel {
            Panel::Inventory => Panel::None,
            Panel::Machine => Panel::None,
            _ => Panel::Inventory,
        };
    }
    if keys.just_pressed(KeyCode::KeyT) {
        ui.panel = match ui.panel {
            Panel::TechTree => Panel::None,
            _ => Panel::TechTree,
        };
    }
    if keys.just_pressed(KeyCode::KeyN) {
        ui.show_network = !ui.show_network;
    }
    // When disconnected, show the menu.
    if *state.get() == ClientState::Disconnected && ui.panel == Panel::None {
        ui.panel = Panel::Menu;
        if ui.address.is_empty() {
            ui.address = net.server.clone().unwrap_or_else(|| "127.0.0.1:5000".into());
        }
        if ui.name.is_empty() {
            ui.name = settings.name.clone();
        }
    }
    let _ = &mut net;
}

fn sync_focus(ui: Res<UiState>, mut input: ResMut<InputState>) {
    let wants_mouse = ui.panel != Panel::None;
    input.ui_focus = wants_mouse;
    if wants_mouse {
        input.captured = false;
    } else if !input.captured {
        input.captured = true;
    }
}

fn draw_ui(
    mut contexts: EguiContexts,
    mut ui: ResMut<UiState>,
    content: Res<GameContent>,
    state: Res<ClientGameState>,
    net: Res<ClientNet>,
    world: Res<ClientWorld>,
    prediction: Res<Prediction>,
    mesher: Res<Mesher>,
    input: Res<InputState>,
    geiger: Res<Geiger>,
    debug: Res<DebugOverlay>,
    client_state: Res<State<ClientState>>,
    mut intents: MessageWriter<Intent>,
    settings: Res<ClientSettings>,
    mut connect: ResMut<ConnectRequest>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let ctx = ctx.clone();

    match ui.panel {
        Panel::Menu => draw_menu(&ctx, &mut ui, &net, &client_state, &mut connect),
        Panel::Inventory => draw_inventory(&ctx, &mut ui, &content, &state, &mut intents),
        Panel::Machine => draw_machine(&ctx, &mut ui, &content, &state, &mut intents),
        Panel::TechTree => draw_tech_tree(&ctx, &mut ui, &content, &state, &mut intents),
        Panel::None => {}
    }

    if ui.panel == Panel::None {
        draw_hud(&ctx, &content, &state, &prediction, &world, &input, &geiger);
        if ui.show_network {
            draw_network_overlay(&ctx, &state);
        }
        if debug.visible {
            let lines = hud::debug_lines(&net, &world, &mesher, &prediction, &client_state);
            draw_debug(&ctx, &lines);
        }
    }
    let _ = settings;
}

fn draw_hud(
    ctx: &egui::Context,
    content: &GameContent,
    state: &ClientGameState,
    prediction: &Prediction,
    world: &ClientWorld,
    input: &InputState,
    geiger: &Geiger,
) {
    let snapshot = hud::snapshot(content, state, prediction, world, input, geiger);
    let screen = ctx.content_rect();

    // Crosshair.
    let center = screen.center();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("crosshair"),
    ));
    let color = if snapshot.too_hard {
        egui::Color32::from_rgb(220, 60, 60)
    } else {
        egui::Color32::from_rgb(230, 230, 230)
    };
    painter.line_segment(
        [center - egui::vec2(8.0, 0.0), center - egui::vec2(2.0, 0.0)],
        (2.0, color),
    );
    painter.line_segment(
        [center + egui::vec2(2.0, 0.0), center + egui::vec2(8.0, 0.0)],
        (2.0, color),
    );
    painter.line_segment(
        [center - egui::vec2(0.0, 8.0), center - egui::vec2(0.0, 2.0)],
        (2.0, color),
    );
    painter.line_segment(
        [center + egui::vec2(0.0, 2.0), center + egui::vec2(0.0, 8.0)],
        (2.0, color),
    );

    if let Some(text) = &snapshot.too_hard_text {
        egui::Area::new(egui::Id::new("too_hard"))
            .fixed_pos(center + egui::vec2(-90.0, 30.0))
            .show(ctx, |ui| {
                ui.colored_label(egui::Color32::from_rgb(240, 90, 90), text);
            });
    }

    // Vitals, bottom left.
    egui::Window::new("vitals")
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -12.0))
        .show(ctx, |ui| {
            ui.label(format!("Health  {:.0}", snapshot.health));
            ui.add(egui::ProgressBar::new(snapshot.health / 100.0).desired_width(140.0));
            ui.label(format!("Oxygen  {:.0}", snapshot.oxygen));
            ui.add(egui::ProgressBar::new(snapshot.oxygen / 100.0).desired_width(140.0));
            if snapshot.dose > 1.0 {
                ui.colored_label(
                    egui::Color32::from_rgb(120, 230, 120),
                    format!("Dose  {:.0}  ({:.1} cps)", snapshot.dose, snapshot.geiger),
                );
            }
        });

    // Depth meter and layer, top left.
    egui::Window::new("depth")
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 12.0))
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(format!("{} m", snapshot.depth)).size(22.0));
            ui.label(snapshot.layer);
        });

    // Hotbar, bottom centre.
    egui::Window::new("hotbar")
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for slot in 0..HOTBAR_SLOTS {
                    let stack = state.inventory.slots.get(slot).and_then(|s| s.stack);
                    let label = match stack {
                        Some(stack) => format!(
                            "{} x{}",
                            content.item_name(stack.item),
                            stack.count
                        ),
                        None => "-".to_string(),
                    };
                    let selected = usize::from(state.inventory.selected) == slot;
                    let text = egui::RichText::new(format!("{}\n{}", slot + 1, label))
                        .size(11.0)
                        .color(if selected {
                            egui::Color32::from_rgb(255, 220, 120)
                        } else {
                            egui::Color32::from_rgb(210, 210, 210)
                        });
                    ui.label(text);
                    ui.separator();
                }
            });
        });

    // Notifications, top right.
    egui::Window::new("notifications")
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
        .show(ctx, |ui| {
            for (text, ttl) in &state.notifications {
                let alpha = (ttl / 4.0).clamp(0.0, 1.0);
                ui.colored_label(
                    egui::Color32::from_rgba_unmultiplied(255, 220, 150, (alpha * 255.0) as u8),
                    text,
                );
            }
        });
}

fn draw_menu(
    ctx: &egui::Context,
    ui: &mut UiState,
    net: &ClientNet,
    client_state: &State<ClientState>,
    request: &mut ConnectRequest,
) {
    egui::Window::new("Deepcore")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui_panel| {
        ui_panel.vertical_centered(|ui_panel| {
            ui_panel.add_space(80.0);
            ui_panel.heading(egui::RichText::new("DEEPCORE").size(42.0));
            ui_panel.label("Dig. Build. Go nuclear.");
            ui_panel.add_space(30.0);
            ui_panel.label(format!("Status: {:?}", client_state.get()));
            if let Some(error) = &net.error {
                ui_panel.colored_label(egui::Color32::from_rgb(240, 100, 100), error);
            }
            ui_panel.add_space(10.0);
            ui_panel.horizontal(|ui_panel| {
                ui_panel.label("Address");
                ui_panel.text_edit_singleline(&mut ui.address);
            });
            ui_panel.horizontal(|ui_panel| {
                ui_panel.label("Name");
                ui_panel.text_edit_singleline(&mut ui.name);
            });
            ui_panel.add_space(10.0);
            if ui_panel.button("Join server").clicked() {
                ui.panel = Panel::None;
                request.0 = Some(ui.address.clone());
            }
            ui_panel.add_space(20.0);
            ui_panel.label(
                "WASD move · Space jump · Shift sprint · LMB mine · RMB interact\n\
                 E inventory · T tech tree · N power overlay · F3 debug · Esc release mouse",
            );
        });
    });
}

fn draw_inventory(
    ctx: &egui::Context,
    ui: &mut UiState,
    content: &GameContent,
    state: &ClientGameState,
    intents: &mut MessageWriter<Intent>,
) {
    egui::Window::new("Inventory")
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |panel| {
        panel.heading("Inventory");
        panel.label("Click a slot to pick it up, click another to move it, right-click to split.");
        panel.separator();
        for row in 0..4 {
            panel.horizontal(|row_ui| {
                for col in 0..9 {
                    let index = row * 9 + col;
                    draw_slot(row_ui, content, state, index, ui, intents);
                }
            });
        }
        panel.separator();
        panel.horizontal(|row_ui| {
            for col in 0..HOTBAR_SLOTS {
                let index = PLAYER_SLOTS - HOTBAR_SLOTS + col;
                draw_slot(row_ui, content, state, index, ui, intents);
            }
        });
        panel.separator();
        if let Some((slot, _)) = ui.drag {
            let item = state.inventory.slots.get(usize::from(slot)).and_then(|s| s.item());
            panel.label(format!("Holding from slot {slot}: {}", item_label(content, item)));
        }
        panel.label(format!(
            "Health {:.0} · Oxygen {:.0} · Dose {:.0}",
            state.vitals.health, state.vitals.oxygen, state.vitals.dose
        ));
    });
}

fn draw_slot(
    ui: &mut egui::Ui,
    content: &GameContent,
    state: &ClientGameState,
    index: usize,
    ui_state: &mut UiState,
    intents: &mut MessageWriter<Intent>,
) {
    let stack = state.inventory.slots.get(index).and_then(|s| s.stack);
    let label = match stack {
        Some(stack) => format!("{}x{}", content.item_name(stack.item), stack.count),
        None => "".to_string(),
    };
    let response = ui.add_sized(
        [78.0, 40.0],
        egui::Button::new(egui::RichText::new(label).size(11.0)),
    );
    if response.clicked() {
        if let Some((from, _)) = ui_state.drag.take() {
            intents.write(Intent::MoveItems {
                from,
                to: index as u16,
                count: u16::MAX,
            });
        } else if stack.is_some() {
            ui_state.drag = Some((index as u16, 1));
        }
    }
    if response.secondary_clicked() {
        if let Some(stack) = stack {
            intents.write(Intent::MoveItems {
                from: index as u16,
                to: index as u16,
                count: stack.count / 2,
            });
        }
    }
}

fn draw_machine(
    ctx: &egui::Context,
    ui: &mut UiState,
    content: &GameContent,
    state: &ClientGameState,
    intents: &mut MessageWriter<Intent>,
) {
    let Some(machine) = &state.open_machine else {
        return;
    };
    let def = &content.machine(machine.machine).def;
    egui::Window::new("Machine")
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |panel| {
        panel.heading(&def.name);
        panel.label(format!("State: {}", machine.idle));
        panel.add(egui::ProgressBar::new(machine.progress).desired_width(240.0));
        panel.separator();

        panel.horizontal(|row| {
            row.group(|inputs| {
                inputs.label("Inputs");
                for (i, slot) in machine.inputs.iter().enumerate() {
                    inputs.label(format!(
                        "{i}: {}",
                        slot.map(|s| format!("{}x{}", content.item_name(s.item), s.count))
                            .unwrap_or_else(|| "-".into())
                    ));
                }
            });
            row.group(|outputs| {
                outputs.label("Outputs");
                for (i, slot) in machine.outputs.iter().enumerate() {
                    outputs.label(format!(
                        "{i}: {}",
                        slot.map(|s| format!("{}x{}", content.item_name(s.item), s.count))
                            .unwrap_or_else(|| "-".into())
                    ));
                }
            });
            row.group(|fuel| {
                fuel.label("Fuel");
                for (i, slot) in machine.fuel.iter().enumerate() {
                    fuel.label(format!(
                        "{i}: {}",
                        slot.map(|s| format!("{}x{}", content.item_name(s.item), s.count))
                            .unwrap_or_else(|| "-".into())
                    ));
                }
            });
        });

        panel.separator();
        panel.label("Recipes");
        for recipe in content.recipes_for(machine.machine) {
            let recipe_def = content.recipe(recipe);
            let unlocked = recipe_def
                .tech
                .is_none_or(|t| state.researched.contains(&t));
            let selected = machine.recipe == Some(recipe);
            let inputs: Vec<String> = recipe_def
                .inputs
                .iter()
                .map(|(i, c)| format!("{c}x {}", content.item_name(*i)))
                .collect();
            let outputs: Vec<String> = recipe_def
                .outputs
                .iter()
                .map(|(i, c)| format!("{c}x {}", content.item_name(*i)))
                .collect();
            let label = format!(
                "{} → {}  ({:.0}s){}",
                inputs.join(" + "),
                outputs.join(" + "),
                recipe_def.def.time,
                if unlocked { "" } else { " [locked]" }
            );
            if panel
                .add_enabled(unlocked, egui::Button::new(label).selected(selected))
                .clicked()
            {
                intents.write(Intent::SetMachineRecipe {
                    machine: machine.pos,
                    recipe: Some(recipe),
                });
                ui.selected_recipe = Some(recipe);
            }
        }
        panel.separator();
        panel.label(format!("Modules: {}", machine.upgrades.len()));
        if panel.button("Close").clicked() {
            ui.panel = Panel::None;
        }
    });
}

fn draw_tech_tree(
    ctx: &egui::Context,
    ui: &mut UiState,
    content: &GameContent,
    state: &ClientGameState,
    intents: &mut MessageWriter<Intent>,
) {
    egui::Window::new("Tech Tree")
        .collapsible(false)
        .anchor(egui::Align2::LEFT_CENTER, egui::vec2(12.0, 0.0))
        .default_width(280.0)
        .show(ctx, |panel| {
        panel.heading("Tech Tree");
        egui::ScrollArea::vertical().show(panel, |scroll| {
            for (index, tech) in content.techs.iter().enumerate() {
                let id = TechId(index as u16);
                let researched = state.researched.contains(&id);
                let parents_done = tech.parents.iter().all(|p| state.researched.contains(p));
                let label = if researched {
                    format!("[done] {}", tech.def.name)
                } else if parents_done {
                    format!("[open] {}", tech.def.name)
                } else {
                    format!("[locked] {}", tech.def.name)
                };
                let selected = ui.selected_tech == Some(id);
                if scroll
                    .selectable_label(selected, format!("Era {} {label}", tech.def.era))
                    .clicked()
                {
                    ui.selected_tech = Some(id);
                }
            }
        });
    });

    egui::Window::new("Tech node")
        .collapsible(false)
        .anchor(egui::Align2::RIGHT_CENTER, egui::vec2(-12.0, 0.0))
        .default_width(360.0)
        .show(ctx, |panel| {
        let Some(id) = ui.selected_tech else {
            panel.label("Select a node on the left.");
            return;
        };
        let tech = content.tech(id);
        panel.heading(&tech.def.name);
        panel.label(format!("Era {}", tech.def.era));
        panel.label(&tech.def.description);
        panel.separator();
        panel.label("Requires:");
        for parent in &tech.parents {
            let done = state.researched.contains(parent);
            panel.label(format!(
                "  {} {}",
                if done { "[x]" } else { "[ ]" },
                content.tech(*parent).def.name
            ));
        }
        panel.separator();
        panel.label("Deliver:");
        let delivered = state
            .research_progress
            .iter()
            .find(|(n, _)| *n == id)
            .map(|(_, items)| items.clone())
            .unwrap_or_default();
        for (item, count) in &tech.cost {
            let have = delivered
                .iter()
                .find(|(i, _)| i == item)
                .map(|(_, c)| *c)
                .unwrap_or(0);
            panel.label(format!(
                "  {}x {}  ({}/{})",
                count,
                content.item_name(*item),
                have,
                count
            ));
        }
        panel.separator();
        panel.label("Unlocks:");
        for unlock in &tech.def.unlocks {
            panel.label(format!("  {unlock}"));
        }
        panel.separator();
        if state.researched.contains(&id) {
            panel.label("Researched");
        } else if panel.button("Deposit from inventory").clicked() {
            intents.write(Intent::ResearchDeposit { node: id });
        }
    });
}

fn draw_network_overlay(ctx: &egui::Context, state: &ClientGameState) {
    egui::Window::new("Power networks")
        .anchor(egui::Align2::RIGHT_CENTER, egui::vec2(-12.0, 0.0))
        .show(ctx, |panel| {
            match &state.networks {
                None => {
                    panel.label("No data yet");
                }
                Some(snapshot) => {
                    for (index, network) in snapshot.stats.networks.iter().enumerate() {
                        panel.label(format!(
                            "#{index} {:?}: supply {:.1} demand {:.1} stored {:.1}/{:.1}",
                            network.kind,
                            network.supply,
                            network.demand,
                            network.stored,
                            network.capacity
                        ));
                    }
                }
            }
        });
}

fn draw_debug(ctx: &egui::Context, lines: &[String]) {
    egui::Window::new("debug")
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 90.0))
        .resizable(false)
        .show(ctx, |panel| {
            for line in lines {
                panel.label(line);
            }
        });
}

/// Opens a machine UI when the server sends its contents.
pub fn open_machine_on_contents(
    mut state: ResMut<ClientGameState>,
    mut ui: ResMut<UiState>,
) {
    if state.open_machine.is_some() {
        ui.panel = Panel::Machine;
    }
}

fn process_connect_request(
    mut request: ResMut<ConnectRequest>,
    mut commands: Commands,
    settings: Res<ClientSettings>,
    content: Res<GameContent>,
    channels: Res<bevy_replicon::shared::backend::channels::RepliconChannels>,
    mut net: ResMut<ClientNet>,
) {
    let Some(address) = request.0.take() else {
        return;
    };
    match net::connect_from_menu(&mut commands, &address, &settings, &content, &channels) {
        Ok(()) => {
            net.server = Some(address);
            net.error = None;
        }
        Err(e) => net.error = Some(e),
    }
}

/// The recipe list for a machine, used by the UI and by tests.
pub fn recipes_for(content: &GameContent, machine: deepcore_shared::content::ids::MachineId) -> Vec<RecipeId> {
    content.recipes_for(machine).collect()
}

/// The role of a machine, used to decide which UI to show.
pub fn role_of(content: &GameContent, machine: deepcore_shared::content::ids::MachineId) -> MachineRole {
    content.machine(machine).def.role
}

/// The name of an item, or "nothing".
pub fn item_label(content: &GameContent, item: Option<ItemId>) -> String {
    match item {
        Some(id) => content.item_name(id).to_string(),
        None => "nothing".into(),
    }
}

/// A category label for the item tooltip.
pub fn category_label(content: &GameContent, item: ItemId) -> &'static str {
    match content.item(item).def.category {
        ItemCategory::Material => "material",
        ItemCategory::Tool(_) => "tool",
        ItemCategory::Block(_) => "block",
        ItemCategory::Machine(_) => "machine",
        ItemCategory::Upgrade(_) => "module",
        ItemCategory::Consumable => "consumable",
        ItemCategory::Fuel { .. } => "fuel",
    }
}

/// Connects to the server at the address typed into the menu.
pub fn menu_connect(
    settings: Res<ClientSettings>,
    content: Res<GameContent>,
    mut commands: Commands,
    mut net: ResMut<ClientNet>,
    ui: Res<UiState>,
    channels: Res<bevy_replicon::shared::backend::channels::RepliconChannels>,
) {
    if let Err(e) = net::connect_from_menu(&mut commands, &ui.address, &settings, &content, &channels)
    {
        net.error = Some(e);
    }
}
