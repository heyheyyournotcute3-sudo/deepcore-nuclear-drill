//! Hazards: vitals, radiation, heat, gas, oxygen, flooding and pickups.

use bevy::prelude::*;

use deepcore_shared::content::ids::BlockId;
use deepcore_shared::hazards::{
    ambient_radiation, dose_rate_at, heat_rate_at, tick_vitals, Environment, Protection,
};
use deepcore_shared::protocol::NetHealth;
use deepcore_shared::voxel::BlockPos;
use deepcore_shared::GameContent;

use crate::sim::components::{PickupQueue, ServerPlayer};
use crate::world::World;

/// Applies one tick of environmental hazards to every player.
pub fn tick_hazards(
    content: Res<GameContent>,
    mut world: ResMut<World>,
    mut players: Query<(&mut ServerPlayer, &mut NetHealth, &deepcore_shared::protocol::NetMovement)>,
    mut pickups: ResMut<PickupQueue>,
) {
    let dt = 1.0 / content.tuning.tick_hz;
    for (mut player, mut health, movement) in players.iter_mut() {
        if player.dead {
            continue;
        }
        let pos = movement.0.position;
        let feet = BlockPos::from_world(pos);
        let eye = [pos[0], pos[1] + 1.62, pos[2]];
        let eye_block = BlockPos::from_world(eye);
        let underwater = content.block(world.block(eye_block)).def.liquid;
        let in_gas = world.gas.contains_key(&eye_block);

        // Radiation from nearby radioactive blocks plus the layer's ambient dose.
        let dose = dose_rate_at(&content, &world.chunks, eye, 8.0)
            + ambient_radiation(&content, eye_block.y.max(0) as u32);
        let heat = heat_rate_at(&content, feet.y.max(0) as u32);

        let env = Environment {
            dose_per_sec: dose,
            heat_per_sec: heat,
            underwater,
            in_gas,
            light: if underwater || feet.y < 100 { 0.2 } else { 1.0 },
        };
        // Gear in the hotbar or backpack applies its protection.
        let protection = protection_of(&content, &player);
        tick_vitals(&content, &mut player.vitals, &env, &protection, dt);
        health.0 = player.vitals.health;
        if !player.vitals.alive() && !player.dead {
            player.dead = true;
            player.mining = None;
        }

        // Pick up items the player is standing on.
        let mut remaining = Vec::new();
        for (item, count, at) in std::mem::take(&mut pickups.items) {
            let dx = at[0] - pos[0];
            let dy = at[1] - pos[1];
            let dz = at[2] - pos[2];
            if dx * dx + dy * dy + dz * dz < 2.25 {
                let result = player.inventory.insert(&content, item, count);
                if result.leftover > 0 {
                    remaining.push((item, result.leftover, at));
                }
                player.inventory_dirty = true;
            } else {
                remaining.push((item, count, at));
            }
        }
        pickups.items = remaining;
        let _ = &mut world;
    }
}

/// The protection factors of the gear a player carries.
pub fn protection_of(content: &GameContent, player: &ServerPlayer) -> Protection {
    let mut protection = Protection::default();
    for slot in &player.inventory.slots {
        let Some(stack) = slot.stack else {
            continue;
        };
        let name = content.item(stack.item).def.id.as_str();
        match name {
            "hazmat_suit" => protection.radiation = 0.2,
            "cooling_suit" => protection.heat = 0.3,
            "respirator" => protection.gas = 0.2,
            _ => {}
        }
    }
    protection
}

/// The block id of water, resolved from content once per call.
pub fn water_block(content: &GameContent) -> BlockId {
    content.block_id("water").unwrap_or(BlockId::AIR)
}
