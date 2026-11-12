//! Authoritative movement: applies queued inputs and sends corrections.

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::movement::{step, Medium};
use deepcore_shared::protocol::{MovementAck, NetMovement};
use deepcore_shared::{GameContent, MovementParams};

use crate::sim::components::ServerPlayer;
use crate::world::World;

/// Applies one fixed tick of movement for every player.
pub fn apply_movement(
    content: Res<GameContent>,
    params: Res<MovementParams>,
    mut world: ResMut<World>,
    mut players: Query<(&mut ServerPlayer, &mut NetMovement, &mut Transform)>,
    mut acks: MessageWriter<ToClients<MovementAck>>,
    sessions: Query<(Entity, &crate::net::handshake::ClientSession)>,
) {
    for (mut player, mut net, mut transform) in players.iter_mut() {
        if player.dead {
            continue;
        }
        let mut state = net.0;
        let inputs = std::mem::take(&mut player.pending_inputs);
        for input in inputs {
            if input.seq <= state.last_seq {
                continue;
            }
            let is_solid = |pos| world.is_solid(&content, pos);
            let medium_at = |p: [f32; 3]| {
                let block = world.block(deepcore_shared::BlockPos::from_world(p));
                if content.block(block).def.liquid {
                    Medium::Water
                } else {
                    Medium::Air
                }
            };
            state = step(&state, &input, &params.0, is_solid, medium_at);
            player.last_seq = input.seq;
        }
        net.0 = state;
        transform.translation =
            Vec3::new(state.position[0], state.position[1], state.position[2]);
    }

    // Acknowledge every client's latest state.
    for (client_entity, session) in sessions.iter() {
        let Some(player_entity) = session.player else {
            continue;
        };
        let Ok((player, net, _)) = players.get(player_entity) else {
            continue;
        };
        let _ = player;
        acks.write(ToClients {
            targets: SendTargets::Single(ClientId::Client(client_entity)),
            message: MovementAck {
                state: net.0,
                last_seq: net.0.last_seq,
                tick: 0,
            },
        });
    }
}
