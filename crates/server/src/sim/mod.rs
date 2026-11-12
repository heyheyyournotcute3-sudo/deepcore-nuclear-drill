//! The server simulation: movement, mining, placing, machines, power and hazards.

pub mod components;
pub mod hazards;
pub mod intents;
pub mod logistics;
pub mod machines;
pub mod mining;
pub mod movement;
pub mod power;

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::protocol::{InventoryUpdate, TechState};

use crate::sim::components::*;
use crate::world::persistence::SaveStore;
use crate::world::World;

/// The simulation plugin: all authoritative game systems.
pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Research>()
            .init_resource::<MachineIndex>()
            .init_resource::<BlockChangeQueue>()
            .init_resource::<PickupQueue>()
            .init_resource::<SimClock>()
            .init_resource::<machines::MachineTimer>()
            .init_resource::<power::NetworkState>()
            .add_systems(Startup, (spawn_world, spawn_bots, spawn_logistics).chain())
            .add_systems(
                FixedUpdate,
                (
                    movement::apply_movement,
                    intents::handle_intents,
                    mining::tick_mining,
                    machines::tick_machines,
                    power::solve_networks,
                    hazards::tick_hazards,
                    intents::send_machine_contents,
                    logistics::tick_logistics,
                    send_inventory_updates,
                    broadcast_tech,
                    tick_clock,
                    autosave,
                )
                    .chain()
                    .run_if(in_state(ServerState::Running)),
            );
    }
}

fn spawn_world(
    mut commands: Commands,
    content: Res<deepcore_shared::GameContent>,
    options: Res<crate::net::ServerOptions>,
) {
    let world = World::new(&content, options.seed);
    info!(
        "world seed {} — spawn at {:?}",
        options.seed,
        world.generator.spawn_point()
    );
    commands.insert_resource(world);
}

fn spawn_logistics(mut commands: Commands, options: Res<crate::net::ServerOptions>) {
    match SaveStore::new(&options.save_dir) {
        Ok(store) => {
            info!("save directory: {}", store.root().display());
            commands.insert_resource(SaveStoreRes(std::sync::Arc::new(store)));
        }
        Err(e) => error!("could not create save directory: {e}"),
    }
}

/// Wraps the save store so it can live in the ECS.
#[derive(Resource, Clone)]
pub struct SaveStoreRes(pub std::sync::Arc<SaveStore>);

fn spawn_bots(mut commands: Commands, options: Res<crate::net::ServerOptions>) {
    for i in 0..options.bots {
        info!("spawning headless bot {i}");
        commands.spawn(Bot { index: i as u32, timer: 0.0 });
    }
}

/// A headless bot used for load and desync testing.
#[derive(Component)]
pub struct Bot {
    /// Bot index.
    pub index: u32,
    /// Timer until the next random action.
    pub timer: f32,
}

/// Autosave every five minutes: serialise on the main thread, write on a background task.
fn autosave(
    clock: Res<SimClock>,
    world: Res<World>,
    machines: Query<&ServerMachine>,
    research: Res<Research>,
    store: Option<Res<SaveStoreRes>>,
    mut last: Local<u32>,
    mut pending: Local<Option<bevy::tasks::Task<()>>>,
) {
    if let Some(task) = pending.as_ref() {
        if task.is_finished() {
            *pending = None;
        }
        return;
    }
    // Every five minutes at 30 Hz.
    if clock.tick.wrapping_sub(*last) < 30 * 300 {
        return;
    }
    *last = clock.tick;
    let Some(store) = store else {
        return;
    };
    let mut save = crate::world::persistence::SaveGame {
        seed: world.seed,
        content_hash: 0,
        edits: world.edits.iter().map(|(p, b)| (*p, *b)).collect(),
        machines: Vec::new(),
        players: Vec::new(),
        researched: research.researched.clone(),
        research_progress: research.progress.clone(),
    };
    for machine in machines.iter() {
        save.machines.push(crate::world::persistence::MachineRecord {
            machine: machine.machine,
            pos: machine.pos,
            facing: machine.facing,
            recipe: machine.state.recipe,
            inputs: machine
                .state
                .inputs
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.stack.map(|st| (i as u8, st.item, st.count)))
                .collect(),
            outputs: machine
                .state
                .outputs
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.stack.map(|st| (i as u8, st.item, st.count)))
                .collect(),
            fuel: machine
                .state
                .fuel
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.stack.map(|st| (i as u8, st.item, st.count)))
                .collect(),
            burn_remaining: machine.state.burn_remaining,
            core_temp: machine.core_temp,
            rod_insertion: machine.rod_insertion,
            scrameed: machine.scrameed,
        });
    }
    let store = store.0.clone();
    let pool = bevy::tasks::AsyncComputeTaskPool::get();
    *pending = Some(pool.spawn(async move {
        if let Err(e) = store.write_save(&save) {
            error!("autosave failed: {e}");
        }
    }));
    info!("autosave started");
}

fn tick_clock(mut clock: ResMut<SimClock>) {
    clock.tick = clock.tick.wrapping_add(1);
}

fn send_inventory_updates(
    mut players: Query<&mut ServerPlayer>,
    sessions: Query<(Entity, &crate::net::handshake::ClientSession)>,
    mut messages: MessageWriter<ToClients<InventoryUpdate>>,
) {
    for (client_entity, session) in sessions.iter() {
        let Some(player_entity) = session.player else {
            continue;
        };
        let Ok(mut player) = players.get_mut(player_entity) else {
            continue;
        };
        if !player.inventory_dirty {
            continue;
        }
        player.inventory_dirty = false;
        messages.write(ToClients {
            targets: SendTargets::Single(ClientId::Client(client_entity)),
            message: InventoryUpdate {
                inventory: player.inventory.clone(),
                vitals: player.vitals,
            },
        });
    }
}

fn broadcast_tech(
    research: Res<Research>,
    clock: Res<SimClock>,
    mut last: Local<u32>,
    mut messages: MessageWriter<ToClients<TechState>>,
) {
    let tick = clock.tick;
    if tick.wrapping_sub(*last) < 30 {
        return;
    }
    *last = tick;
    messages.write(ToClients {
        targets: SendTargets::All,
        message: TechState {
            researched: research.researched.clone(),
            progress: research.progress.clone(),
        },
    });
}
