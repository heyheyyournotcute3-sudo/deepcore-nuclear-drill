//! Steam and electricity networks.
//!
//! Topology is rebuilt only when a pipe, cable or machine is placed or removed; each tick the
//! solver only sums supply and demand per network.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::content::ids::MachineId;
use crate::voxel::chunk::BlockPos;

/// Which kind of network a node belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkKind {
    /// Steam carried in pipes.
    Steam,
    /// Electricity carried in cables.
    Electric,
}

/// A node in a network: a pipe/cable block or a machine that produces or consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkNode {
    /// Position of the node.
    pub pos: BlockPos,
    /// Network kind.
    pub kind: NetworkKind,
    /// Machine attached at this position, if any.
    pub machine: Option<MachineId>,
}

/// A connected network with its cached totals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Network {
    /// Network kind.
    pub kind: NetworkKind,
    /// Nodes in the network, in stable order.
    pub nodes: Vec<NetworkNode>,
    /// Steam units per second produced, or kilowatts produced.
    pub supply: f32,
    /// Steam units per second demanded, or kilowatts demanded.
    pub demand: f32,
    /// Stored energy in kWh (electric only).
    pub stored: f32,
    /// Storage capacity in kWh (electric only).
    pub capacity: f32,
}

impl Network {
    /// Fraction of demand that can be met, 0..=1.
    pub fn satisfaction(&self) -> f32 {
        if self.demand <= 0.0 {
            1.0
        } else {
            (self.supply / self.demand).clamp(0.0, 1.0)
        }
    }
}

/// Per-machine network statistics for the overlay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkStats {
    /// Networks, keyed by a stable network id.
    pub networks: Vec<Network>,
}

/// An index of network membership by position.
#[derive(Debug, Default, Clone)]
pub struct NetworkIndex {
    /// Network id per position.
    membership: HashMap<BlockPos, usize>,
    /// Networks by id.
    pub networks: Vec<Network>,
    /// Kind of each position, for topology rebuilds.
    kinds: HashMap<BlockPos, NetworkKind>,
}

/// A producer or consumer attached to a network.
#[derive(Debug, Clone, Copy)]
pub struct Attached {
    /// Position of the machine.
    pub pos: BlockPos,
    /// Machine type.
    pub machine: MachineId,
    /// Positive for producers, negative for consumers.
    pub rate: f32,
}

impl NetworkIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a conductor (pipe or cable) at a position.
    pub fn add_conductor(&mut self, pos: BlockPos, kind: NetworkKind) {
        self.kinds.insert(pos, kind);
    }

    /// Removes a conductor.
    pub fn remove_conductor(&mut self, pos: BlockPos) {
        self.kinds.remove(&pos);
        if let Some(id) = self.membership.remove(&pos) {
            if let Some(network) = self.networks.get_mut(id) {
                network.nodes.retain(|n| n.pos != pos);
            }
        }
    }

    /// Rebuilds the topology from scratch. Called only on placement/removal, never per tick.
    pub fn rebuild(&mut self, attached: &[Attached]) {
        self.membership.clear();
        self.networks.clear();

        let mut visited: Vec<BlockPos> = Vec::new();
        let mut positions: Vec<(BlockPos, NetworkKind)> =
            self.kinds.iter().map(|(p, k)| (*p, *k)).collect();
        positions.sort_by_key(|(p, _)| (p.y, p.z, p.x));

        for (start, kind) in positions {
            if visited.contains(&start) {
                continue;
            }
            let network_id = self.networks.len();
            let mut nodes = Vec::new();
            let mut stack = vec![start];
            visited.push(start);
            while let Some(pos) = stack.pop() {
                self.membership.insert(pos, network_id);
                let machine = attached
                    .iter()
                    .find(|a| a.pos == pos)
                    .map(|a| a.machine);
                nodes.push(NetworkNode { pos, kind, machine });
                for delta in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                    let neighbour = pos.offset(delta.0, delta.1, delta.2);
                    if self.kinds.get(&neighbour) == Some(&kind) && !visited.contains(&neighbour) {
                        visited.push(neighbour);
                        stack.push(neighbour);
                    }
                }
            }
            nodes.sort_by_key(|n| (n.pos.y, n.pos.z, n.pos.x));
            self.networks.push(Network {
                kind,
                nodes,
                supply: 0.0,
                demand: 0.0,
                stored: 0.0,
                capacity: 0.0,
            });
        }
    }

    /// Network id containing a position.
    pub fn network_of(&self, pos: BlockPos) -> Option<usize> {
        self.membership.get(&pos).copied()
    }

    /// The network containing a position.
    pub fn network_at(&self, pos: BlockPos) -> Option<&Network> {
        self.network_of(pos).and_then(|id| self.networks.get(id))
    }

    /// Clears cached totals before a solve.
    pub fn clear_totals(&mut self) {
        for network in &mut self.networks {
            network.supply = 0.0;
            network.demand = 0.0;
        }
    }

    /// Adds a producer to the network containing `pos`.
    pub fn add_supply(&mut self, pos: BlockPos, amount: f32) {
        if let Some(id) = self.network_of(pos) {
            if let Some(network) = self.networks.get_mut(id) {
                network.supply += amount;
            }
        }
    }

    /// Adds a consumer to the network containing `pos`.
    pub fn add_demand(&mut self, pos: BlockPos, amount: f32) {
        if let Some(id) = self.network_of(pos) {
            if let Some(network) = self.networks.get_mut(id) {
                network.demand += amount;
            }
        }
    }

    /// Sets battery storage for the network containing `pos`.
    pub fn add_storage(&mut self, pos: BlockPos, stored: f32, capacity: f32) {
        if let Some(id) = self.network_of(pos) {
            if let Some(network) = self.networks.get_mut(id) {
                network.stored += stored;
                network.capacity += capacity;
            }
        }
    }

    /// Resolves the electric networks: batteries cover deficits and absorb surplus.
    ///
    /// Returns the power factor for each position (0..=1).
    pub fn solve_electric(&mut self, dt_hours: f32) -> HashMap<BlockPos, f32> {
        let mut factors = HashMap::new();
        for network in &mut self.networks {
            if network.kind != NetworkKind::Electric {
                continue;
            }
            let mut factor = if network.demand <= 0.0 {
                1.0
            } else {
                (network.supply / network.demand).clamp(0.0, 1.0)
            };
            // Draw from storage to cover the deficit.
            if factor < 1.0 && network.capacity > 0.0 {
                let deficit_kw = network.demand * (1.0 - factor);
                let available_kw = network.stored / dt_hours.max(1e-6);
                let cover = deficit_kw.min(available_kw);
                if cover > 0.0 {
                    network.stored = (network.stored - cover * dt_hours).max(0.0);
                    factor += cover / network.demand;
                    factor = factor.min(1.0);
                }
            }
            // Charge from the surplus.
            if factor >= 1.0 && network.supply > network.demand && network.capacity > 0.0 {
                let surplus = network.supply - network.demand;
                let room = network.capacity - network.stored;
                let charge = (surplus * dt_hours).min(room);
                network.stored += charge.max(0.0);
            }
            for node in &network.nodes {
                factors.insert(node.pos, factor);
            }
        }
        factors
    }

    /// Resolves the steam networks: supply is shared proportionally across consumers.
    pub fn solve_steam(&mut self) -> HashMap<BlockPos, f32> {
        let mut factors = HashMap::new();
        for network in &self.networks {
            if network.kind != NetworkKind::Steam {
                continue;
            }
            let factor = network.satisfaction();
            for node in &network.nodes {
                factors.insert(node.pos, factor);
            }
        }
        factors
    }

    /// A snapshot for the network overlay.
    pub fn stats(&self) -> NetworkStats {
        NetworkStats { networks: self.networks.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_pipes_form_one_network() {
        let mut index = NetworkIndex::new();
        for x in 0..5 {
            index.add_conductor(BlockPos::new(x, 0, 0), NetworkKind::Steam);
        }
        index.rebuild(&[]);
        assert_eq!(index.networks.len(), 1);
        assert_eq!(index.networks[0].nodes.len(), 5);
    }

    #[test]
    fn gaps_split_networks() {
        let mut index = NetworkIndex::new();
        index.add_conductor(BlockPos::new(0, 0, 0), NetworkKind::Electric);
        index.add_conductor(BlockPos::new(2, 0, 0), NetworkKind::Electric);
        index.rebuild(&[]);
        assert_eq!(index.networks.len(), 2);
    }

    #[test]
    fn brownout_scales_consumers() {
        let mut index = NetworkIndex::new();
        index.add_conductor(BlockPos::new(0, 0, 0), NetworkKind::Electric);
        index.rebuild(&[]);
        index.add_supply(BlockPos::new(0, 0, 0), 50.0);
        index.add_demand(BlockPos::new(0, 0, 0), 100.0);
        let factors = index.solve_electric(1.0 / 3600.0);
        let f = factors[&BlockPos::new(0, 0, 0)];
        assert!((f - 0.5).abs() < 1e-6, "factor = {f}");
    }

    #[test]
    fn batteries_cover_deficit() {
        let mut index = NetworkIndex::new();
        index.add_conductor(BlockPos::new(0, 0, 0), NetworkKind::Electric);
        index.rebuild(&[]);
        index.add_supply(BlockPos::new(0, 0, 0), 0.0);
        index.add_demand(BlockPos::new(0, 0, 0), 100.0);
        index.add_storage(BlockPos::new(0, 0, 0), 50.0, 100.0);
        let factors = index.solve_electric(0.1);
        assert!(factors[&BlockPos::new(0, 0, 0)] > 0.9);
        assert!(index.networks[0].stored < 50.0);
    }

    #[test]
    fn surplus_charges_batteries() {
        let mut index = NetworkIndex::new();
        index.add_conductor(BlockPos::new(0, 0, 0), NetworkKind::Electric);
        index.rebuild(&[]);
        index.add_supply(BlockPos::new(0, 0, 0), 100.0);
        index.add_demand(BlockPos::new(0, 0, 0), 20.0);
        index.add_storage(BlockPos::new(0, 0, 0), 0.0, 100.0);
        index.solve_electric(1.0);
        assert!(index.networks[0].stored > 0.0);
    }

    #[test]
    fn steam_shares_proportionally() {
        let mut index = NetworkIndex::new();
        index.add_conductor(BlockPos::new(0, 0, 0), NetworkKind::Steam);
        index.rebuild(&[]);
        index.add_supply(BlockPos::new(0, 0, 0), 5.0);
        index.add_demand(BlockPos::new(0, 0, 0), 10.0);
        let factors = index.solve_steam();
        assert!((factors[&BlockPos::new(0, 0, 0)] - 0.5).abs() < 1e-6);
    }
}
