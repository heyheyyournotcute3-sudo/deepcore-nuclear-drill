//! Movement prediction and reconciliation.

use std::collections::VecDeque;

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use deepcore_shared::movement::{rotate_input, step, Medium, MoveInput, MovementState};
use deepcore_shared::protocol::{Intent, MovementAck};
use deepcore_shared::{GameContent, MovementParams};

use crate::input::{InputState, PlayerCamera};
use crate::world::ClientWorld;

/// A pending input that the server has not acknowledged yet.
#[derive(Debug, Clone, Copy)]
pub struct PendingInput {
    /// The input.
    pub input: MoveInput,
    /// Position after applying it locally.
    pub predicted: MovementState,
}

/// The local player's prediction state.
#[derive(Resource)]
pub struct Prediction {
    /// Our best guess of the authoritative state.
    pub state: MovementState,
    /// Inputs sent but not yet acknowledged.
    pub pending: VecDeque<PendingInput>,
    /// Sequence counter.
    pub seq: u32,
    /// Smoothed visual offset applied on top of the state.
    pub smoothing: [f32; 3],
    /// Number of corrections applied since the last report.
    pub corrections: u32,
    /// Whether we have received our first authoritative state.
    pub initialized: bool,
}

impl Default for Prediction {
    fn default() -> Self {
        Self {
            state: MovementState::at_spawn([512.0, 490.0, 512.0]),
            pending: VecDeque::new(),
            seq: 0,
            smoothing: [0.0, 0.0, 0.0],
            corrections: 0,
            initialized: false,
        }
    }
}

/// The prediction plugin.
pub struct PredictionPlugin;

impl Plugin for PredictionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Prediction>()
            .add_systems(
                Update,
                (send_inputs, reconcile, apply_to_camera).chain(),
            );
    }
}

/// Applies the local input, sends it to the server and stores it for reconciliation.
fn send_inputs(
    input: Res<InputState>,
    keys: Res<ButtonInput<KeyCode>>,
    content: Res<GameContent>,
    params: Res<MovementParams>,
    world: Res<ClientWorld>,
    mut prediction: ResMut<Prediction>,
    mut intents: MessageWriter<Intent>,
) {
    if !prediction.initialized {
        prediction.state.yaw = input.yaw;
        prediction.state.pitch = input.pitch;
        return;
    }
    prediction.seq = prediction.seq.wrapping_add(1);
    let dir = rotate_input(input.movement[0], input.movement[1], input.yaw);
    let move_input = MoveInput {
        seq: prediction.seq,
        dir,
        jump: keys.pressed(KeyCode::Space),
        sprint: keys.pressed(KeyCode::ShiftLeft) && input.movement[1] > 0.0,
        crouch: keys.pressed(KeyCode::ControlLeft),
        yaw: input.yaw,
        pitch: input.pitch,
    };
    let is_solid = |pos| world.is_solid(&content, pos);
    let medium_at = |p: [f32; 3]| {
        let block = world.block(deepcore_shared::BlockPos::from_world(p));
        if content.block(block).def.liquid {
            Medium::Water
        } else {
            Medium::Air
        }
    };
    let next = step(&prediction.state, &move_input, &params.0, is_solid, medium_at);
    prediction.state = next;
    prediction.pending.push_back(PendingInput { input: move_input, predicted: next });
    while prediction.pending.len() > 128 {
        prediction.pending.pop_front();
    }
    intents.write(Intent::Move(move_input));
}

/// Rewinds to the server's state and replays unacknowledged inputs.
fn reconcile(
    mut acks: MessageReader<MovementAck>,
    mut prediction: ResMut<Prediction>,
    content: Res<GameContent>,
    params: Res<MovementParams>,
    world: Res<ClientWorld>,
) {
    let mut latest: Option<MovementAck> = None;
    for ack in acks.read() {
        latest = Some(*ack);
    }
    let Some(ack) = latest else {
        return;
    };
    prediction.initialized = true;

    // Drop acknowledged inputs.
    while let Some(front) = prediction.pending.front() {
        if front.input.seq <= ack.last_seq {
            prediction.pending.pop_front();
        } else {
            break;
        }
    }

    let before = prediction.state.position;
    let mut state = ack.state;
    let is_solid = |pos| world.is_solid(&content, pos);
    let medium_at = |p: [f32; 3]| {
        let block = world.block(deepcore_shared::BlockPos::from_world(p));
        if content.block(block).def.liquid {
            Medium::Water
        } else {
            Medium::Air
        }
    };
    for pending in &prediction.pending {
        state = step(&state, &pending.input, &params.0, is_solid, medium_at);
    }
    let after = state.position;
    let dx = before[0] - after[0];
    let dy = before[1] - after[1];
    let dz = before[2] - after[2];
    let error = (dx * dx + dy * dy + dz * dz).sqrt();
    if error > 0.3 {
        // Large correction: snap and count it for the debug overlay.
        prediction.corrections += 1;
        prediction.smoothing = [0.0, 0.0, 0.0];
    } else if error > 0.001 {
        // Small correction: smooth it out over ~100 ms.
        prediction.smoothing[0] += dx;
        prediction.smoothing[1] += dy;
        prediction.smoothing[2] += dz;
    }
    prediction.state = state;
}

/// Moves the camera to the predicted eye position.
fn apply_to_camera(
    mut prediction: ResMut<Prediction>,
    params: Res<MovementParams>,
    mut cameras: Query<&mut Transform, With<PlayerCamera>>,
    time: Res<Time>,
) {
    if !prediction.initialized {
        return;
    }
    let decay = (-time.delta_secs() / 0.1).exp();
    for value in &mut prediction.smoothing {
        *value *= decay;
        if value.abs() < 1e-4 {
            *value = 0.0;
        }
    }
    let Ok(mut transform) = cameras.single_mut() else {
        return;
    };
    let state = prediction.state;
    transform.translation = Vec3::new(
        state.position[0] + prediction.smoothing[0],
        state.position[1] + params.0.eye_height + prediction.smoothing[1],
        state.position[2] + prediction.smoothing[2],
    );
    transform.rotation = Quat::from_euler(EulerRot::YXZ, state.yaw, state.pitch, 0.0);
}
