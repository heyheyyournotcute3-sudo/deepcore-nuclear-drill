//! Power: steam and electric network solving each tick.

use std::collections::HashMap;

use bevy::prelude::*;

use deepcore_shared::content::defs::{EnergyKind, MachineRole};
use deepcore_shared::power::NetworkIndex;
use deepcore_shared::voxel::BlockPos;
use deepcore_shared::GameContent;

use crate::sim::components::ServerMachine;

/// The solved power state for the current tick.
#[derive(Resource, Default)]
pub struct NetworkState {
    /// Power factor per machine position.
    pub factors: HashMap<BlockPos, f32>,
    /// Steam produced this tick, per position.
    pub steam_supply: HashMap<BlockPos, f32>,
    /// Steam demanded this tick, per position.
    pub steam_demand: HashMap<BlockPos, f32>,
    /// Electricity produced this tick (kWh), per position.
    pub electric_supply: HashMap<BlockPos, f32>,
    /// Electricity demanded this tick (kWh), per position.
    pub electric_demand: HashMap<BlockPos, f32>,
    /// Stored energy per position.
    pub stored: HashMap<BlockPos, f32>,
    /// The network index, rebuilt on topology change.
    pub index: NetworkIndex,
    /// Whether the topology needs rebuilding.
    pub dirty: bool,
}

impl NetworkState {
    /// Adds steam to the pool at a position.
    pub fn add_supply(&mut self, pos: BlockPos, amount: f32) {
        *self.steam_supply.entry(pos).or_insert(0.0) += amount;
    }

    /// Steam available at a position this tick.
    pub fn steam_available(&self, pos: BlockPos) -> f32 {
        let supply: f32 = self.steam_supply.values().sum();
        let demand: f32 = self.steam_demand.values().sum();
        if demand <= 0.0 {
            return supply;
        }
        supply * (1.0 - (demand / supply.max(1e-6)).min(1.0))
    }

    /// Consumes steam at a position.
    pub fn consume_steam(&mut self, pos: BlockPos, amount: f32) {
        *self.steam_demand.entry(pos).or_insert(0.0) += amount;
    }

    /// Adds electricity (in kWh for this tick) at a position.
    pub fn add_electric(&mut self, pos: BlockPos, kwh: f32) {
        *self.electric_supply.entry(pos).or_insert(0.0) += kwh;
    }

    /// Registers battery storage at a position.
    pub fn add_storage(&mut self, pos: BlockPos, capacity: f32) {
        self.stored.entry(pos).or_insert(capacity);
    }

    /// Number of centrifuges stacked above a position (the cascade length).
    pub fn cascade_length(&self, pos: BlockPos) -> u32 {
        self.index
            .networks
            .iter()
            .flat_map(|n| n.nodes.iter())
            .filter(|node| {
                node.pos.x == pos.x
                    && node.pos.z == pos.z
                    && node.pos.y >= pos.y
                    && node.pos.y < pos.y + 16
            })
            .count() as u32
    }
}

/// Rebuilds network topology and solves supply/demand.
pub fn solve_networks(
    content: Res<GameContent>,
    mut machines: Query<&mut ServerMachine>,
    mut network: ResMut<NetworkState>,
) {
    if network.dirty {
        let mut attached = Vec::new();
        for machine in machines.iter() {
            let def = &content.machine(machine.machine).def;
            let produces = matches!(
                def.role,
                MachineRole::Boiler { .. }
                    | MachineRole::SteamEngine { .. }
                    | MachineRole::Generator { .. }
                    | MachineRole::Battery { .. }
                    | MachineRole::Reactor
            );
            let consumes = !matches!(
                def.role,
                MachineRole::Boiler { .. }
                    | MachineRole::Generator { .. }
                    | MachineRole::Battery { .. }
                    | MachineRole::ResearchDesk
                    | MachineRole::Lamp
                    | MachineRole::Storage
            );
            if produces || consumes {
                attached.push(deepcore_shared::power::Attached {
                    pos: machine.pos,
                    machine: machine.machine,
                    rate: 0.0,
                });
            }
        }
        network.index.rebuild(&attached);
        network.dirty = false;
    }

    // Sum this tick's production and demand.
    let mut demand = HashMap::new();
    for machine in machines.iter() {
        let def = &content.machine(machine.machine).def;
        if let EnergyKind::Electric { kw } = def.energy {
            let load = kw * machine.state.power_multiplier(&content);
            *demand.entry(machine.pos).or_insert(0.0f32) += load / 3600.0 * (1.0 / content.tuning.tick_hz) * 3600.0;
        }
    }
    network.electric_demand = demand;

    // Power factor per machine: supply / demand within its network.
    let mut factors = HashMap::new();
    for machine in machines.iter() {
        let pos = machine.pos;
        let network_id = network.index.network_of(pos);
        let (supply, total_demand) = match network_id {
            Some(id) => {
                let net = &network.index.networks[id];
                (net.supply, net.demand)
            }
            None => (1.0, 0.0),
        };
        let factor = if total_demand <= 0.0 {
            1.0
        } else {
            (supply / total_demand).clamp(0.0, 1.0)
        };
        factors.insert(pos, factor);
    }
    network.factors = factors;
}
