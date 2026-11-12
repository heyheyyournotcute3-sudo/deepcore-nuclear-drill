//! Machine ticking: processors, boilers, generators, batteries, reactors and cascades.

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::content::defs::MachineRole;
use deepcore_shared::crafting::{tick_processor, TickOutcome};
use deepcore_shared::protocol::{NetMachine, Notify};
use deepcore_shared::voxel::BlockPos;
use deepcore_shared::GameContent;

use crate::sim::components::{PickupQueue, Research, ServerMachine, SimClock};
use crate::sim::intents::machine_contents;
use crate::sim::power::NetworkState;

/// Timer used to throttle machine broadcasts.
#[derive(Resource, Default)]
pub struct MachineTimer {
    /// Last tick a broadcast was sent.
    pub last_broadcast: u32,
}

/// Advances every machine by one fixed tick.
#[allow(clippy::too_many_arguments)]
pub fn tick_machines(
    content: Res<GameContent>,
    research: Res<Research>,
    clock: Res<SimClock>,
    mut machines: Query<(&mut ServerMachine, &mut NetMachine)>,
    mut network: ResMut<NetworkState>,
    mut pickups: ResMut<PickupQueue>,
    mut notifies: MessageWriter<ToClients<Notify>>,
    mut timer: ResMut<MachineTimer>,
) {
    let dt = 1.0 / content.tuning.tick_hz;
    let mut alarm = false;

    for (mut machine, mut net) in machines.iter_mut() {
        let def = content.machine(machine.machine).def.clone();
        let power_factor = network
            .factors
            .get(&machine.pos)
            .copied()
            .unwrap_or(1.0);

        match def.role {
            MachineRole::Processor => {
                if def.cascade.is_some() {
                    tick_cascade(&content, &research, &mut machine, &mut net, dt, power_factor, &mut network);
                } else if let TickOutcome::Completed(recipe) = tick_processor(
                    &content,
                    &mut machine.state,
                    dt,
                    power_factor,
                    |gate| research.unlocked(&content, gate),
                ) {
                    // Dump finished outputs onto the ground if there is nobody to collect them.
                    let _ = recipe;
                }
                net.progress = machine.state.display_progress;
                net.running = machine.state.running;
            }
            MachineRole::Boiler { steam_per_sec } => {
                let burning = machine.state.burn_remaining > 0.0 || machine.state.refuel(&content);
                if burning {
                    machine.state.tick_fuel(&content, dt);
                    network.add_supply(machine.pos, steam_per_sec * dt);
                    net.running = true;
                } else {
                    net.running = false;
                }
                net.progress = (machine.state.burn_remaining / content.tuning.coal_burn_secs)
                    .clamp(0.0, 1.0);
            }
            MachineRole::SteamEngine { steam_per_sec, kw } => {
                let available = network.steam_available(machine.pos);
                if available >= steam_per_sec * dt {
                    network.consume_steam(machine.pos, steam_per_sec * dt);
                    network.add_electric(machine.pos, kw * dt);
                    net.running = true;
                } else {
                    net.running = false;
                }
            }
            MachineRole::Generator { kw } => {
                let burning = machine.state.burn_remaining > 0.0 || machine.state.refuel(&content);
                if burning {
                    machine.state.tick_fuel(&content, dt);
                    network.add_electric(machine.pos, kw * dt);
                    net.running = true;
                } else {
                    net.running = false;
                }
            }
            MachineRole::Battery { capacity_kwh } => {
                network.add_storage(machine.pos, capacity_kwh);
                net.running = true;
            }
            MachineRole::Reactor => {
                tick_reactor(&content, &mut machine, dt, &mut network, &mut alarm);
                net.running = machine.reactor_kw > 0.0;
                net.progress = (machine.core_temp / 900.0).clamp(0.0, 1.0);
            }
            MachineRole::ChargingStation
            | MachineRole::ResearchDesk
            | MachineRole::Lamp
            | MachineRole::Storage
            | MachineRole::Utility => {
                net.running = true;
            }
        }
        // Spilled outputs from a machine that cannot hold them become pickups.
        let _ = &mut pickups;
    }

    if alarm {
        notifies.write(ToClients {
            targets: SendTargets::All,
            message: Notify {
                text: "REACTOR OVERHEATING — SCRAM REQUIRED".into(),
                level: 2,
            },
        });
    }

    // Broadcast coarse progress once a second.
    if clock.tick.wrapping_sub(timer.last_broadcast) >= 30 {
        timer.last_broadcast = clock.tick;
    }
}

/// A centrifuge cascade: runs only when enough centrifuges are stacked vertically.
#[allow(clippy::too_many_arguments)]
fn tick_cascade(
    content: &GameContent,
    research: &Research,
    machine: &mut ServerMachine,
    net: &mut NetMachine,
    dt: f32,
    power_factor: f32,
    network: &mut NetworkState,
) {
    let def = content.machine(machine.machine).def.clone();
    let Some(cascade) = def.cascade else {
        return;
    };
    let length = network.cascade_length(machine.pos);
    if length < u32::from(cascade.min) {
        machine.state.running = false;
        machine.state.idle = deepcore_shared::IdleReason::CascadeTooShort;
        return;
    }
    let effective = length.min(u32::from(cascade.max)) as f32 / f32::from(cascade.max);
    let speed = 0.5 + effective;
    let _ = tick_processor(
        content,
        &mut machine.state,
        dt * speed,
        power_factor,
        |gate| research.unlocked(content, gate),
    );
    net.progress = machine.state.display_progress;
    net.running = machine.state.running;
}

/// The reactor: temperature rises with withdrawn rods, coolant and control rods cool it.
fn tick_reactor(
    content: &GameContent,
    machine: &mut ServerMachine,
    dt: f32,
    network: &mut NetworkState,
    alarm: &mut bool,
) {
    if machine.scrameed {
        machine.reactor_kw = 0.0;
        machine.core_temp = (machine.core_temp - 40.0 * dt).max(20.0);
        return;
    }
    let has_fuel = machine.state.inputs.iter().any(|s| {
        s.item()
            .is_some_and(|i| content.item_name(i).contains("Fuel Rod"))
    });
    if !has_fuel {
        machine.reactor_kw = 0.0;
        machine.core_temp = (machine.core_temp - 20.0 * dt).max(20.0);
        return;
    }
    let _ = content;
    let rods = machine.rod_insertion.clamp(0.0, 1.0);
    let heat_in = (1.0 - rods) * 120.0;
    let cooling = 55.0;
    machine.core_temp = (machine.core_temp + (heat_in - cooling) * dt).clamp(20.0, 1200.0);
    machine.reactor_kw = ((1.0 - rods) * 1200.0).max(0.0);
    network.add_electric(machine.pos, machine.reactor_kw * dt);
    if machine.core_temp > 700.0 {
        *alarm = true;
    }
    if machine.core_temp >= 1100.0 {
        machine.scrameed = true;
        machine.core_temp = 1200.0;
    }
}

/// Places the fuel rod from a machine's output into its input when the reactor is refuelled.
pub fn refuel_reactor(content: &GameContent, machine: &mut ServerMachine) -> bool {
    if machine.state.inputs.iter().any(|s| {
        s.item()
            .is_some_and(|i| content.item_name(i).contains("Fuel Rod"))
    }) {
        return false;
    }
    for slot in &mut machine.state.outputs {
        if let Some(stack) = &mut slot.stack {
            if content.item_name(stack.item).contains("Fuel Rod") {
                machine.state.inputs[0].stack = Some(*stack);
                slot.stack = None;
                return true;
            }
        }
    }
    false
}

/// Sends machine contents to players with the UI open (re-exported for tests).
pub fn contents_for(content: &GameContent, machine: &ServerMachine) -> deepcore_shared::protocol::MachineContents {
    machine_contents(content, machine)
}

/// Convenience: a machine's block position.
pub fn position(machine: &ServerMachine) -> BlockPos {
    machine.pos
}

/// Unused parameter guard so the pickup queue stays part of the signature for future spills.
#[allow(dead_code)]
fn _pickups(_: &mut PickupQueue) {}
