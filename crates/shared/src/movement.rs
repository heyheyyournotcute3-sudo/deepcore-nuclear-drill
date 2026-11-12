//! Deterministic AABB-vs-voxel player physics, shared by the server and the client's prediction.
//!
//! The controller is a pure function of the previous state and a [`MoveInput`], stepped at a fixed
//! timestep. No randomness, no floating point ordering surprises: both sides run the same code
//! and get the same result.

use serde::{Deserialize, Serialize};

use crate::voxel::chunk::BlockPos;

/// A single frame of player input.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MoveInput {
    /// Sequence number, used for reconciliation.
    pub seq: u32,
    /// Movement direction in world space (already rotated by yaw), length ≤ 1.
    pub dir: [f32; 2],
    /// Whether the player wants to jump.
    pub jump: bool,
    /// Whether the player wants to sprint.
    pub sprint: bool,
    /// Whether the player wants to crouch.
    pub crouch: bool,
    /// Yaw in radians.
    pub yaw: f32,
    /// Pitch in radians.
    pub pitch: f32,
}

impl MoveInput {
    /// An input with no movement.
    pub fn idle(seq: u32, yaw: f32, pitch: f32) -> Self {
        Self { seq, dir: [0.0, 0.0], jump: false, sprint: false, crouch: false, yaw, pitch }
    }
}

/// Movement constants, taken from `tuning.ron`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MoveParams {
    /// Gravity in blocks per second squared.
    pub gravity: f32,
    /// Walk speed.
    pub walk_speed: f32,
    /// Sprint speed.
    pub sprint_speed: f32,
    /// Crouch speed.
    pub crouch_speed: f32,
    /// Jump velocity.
    pub jump_speed: f32,
    /// Player half-width.
    pub radius: f32,
    /// Player height.
    pub height: f32,
    /// Eye height above the feet.
    pub eye_height: f32,
    /// Fixed timestep in seconds.
    pub dt: f32,
    /// Maximum step-up height (0 disables auto step).
    pub step_height: f32,
    /// Terminal fall speed.
    pub terminal_speed: f32,
}

impl Default for MoveParams {
    fn default() -> Self {
        Self {
            gravity: 28.0,
            walk_speed: 4.5,
            sprint_speed: 7.0,
            crouch_speed: 2.0,
            jump_speed: 8.4,
            radius: 0.3,
            height: 1.8,
            eye_height: 1.62,
            dt: 1.0 / 30.0,
            step_height: 0.55,
            terminal_speed: 40.0,
        }
    }
}

/// The physical state of a player, replicated and predicted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MovementState {
    /// Feet position in world space.
    pub position: [f32; 3],
    /// Velocity in blocks per second.
    pub velocity: [f32; 3],
    /// Yaw in radians.
    pub yaw: f32,
    /// Pitch in radians.
    pub pitch: f32,
    /// Whether the player is standing on solid ground.
    pub on_ground: bool,
    /// Whether the player is in water.
    pub in_water: bool,
    /// Last processed input sequence number.
    pub last_seq: u32,
}

impl MovementState {
    /// A player standing at a spawn point.
    pub fn at_spawn(position: [f32; 3]) -> Self {
        Self {
            position,
            velocity: [0.0, 0.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            in_water: false,
            last_seq: 0,
        }
    }

    /// Eye position in world space.
    pub fn eye(&self, params: &MoveParams) -> [f32; 3] {
        [
            self.position[0],
            self.position[1] + params.eye_height,
            self.position[2],
        ]
    }

    /// Forward direction vector from yaw and pitch.
    pub fn look_dir(&self) -> [f32; 3] {
        let cos_pitch = self.pitch.cos();
        [
            -self.yaw.sin() * cos_pitch,
            self.pitch.sin(),
            -self.yaw.cos() * cos_pitch,
        ]
    }
}

/// What the player is standing in, used by hazards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    /// Air.
    Air,
    /// Water.
    Water,
}

/// Steps the player one fixed tick.
///
/// `is_solid` must report whether a block blocks movement (solid terrain, not air or water).
/// `medium_at` reports the medium at a point.
pub fn step(
    state: &MovementState,
    input: &MoveInput,
    params: &MoveParams,
    is_solid: impl Fn(BlockPos) -> bool,
    medium_at: impl Fn([f32; 3]) -> Medium,
) -> MovementState {
    let mut next = *state;
    next.yaw = input.yaw;
    next.pitch = input.pitch.clamp(-1.5533, 1.5533);
    next.last_seq = input.seq;

    let eye = next.eye(params);
    let medium = medium_at(eye);
    next.in_water = medium == Medium::Water;

    let mut wish = [input.dir[0], input.dir[1]];
    let wish_len = (wish[0] * wish[0] + wish[1] * wish[1]).sqrt();
    if wish_len > 1.0 {
        wish[0] /= wish_len;
        wish[1] /= wish_len;
    }

    let speed = if next.in_water {
        if input.sprint { params.walk_speed * 0.8 } else { params.walk_speed * 0.5 }
    } else if input.crouch {
        params.crouch_speed
    } else if input.sprint {
        params.sprint_speed
    } else {
        params.walk_speed
    };

    // Horizontal acceleration: snap to the wish velocity for predictable movement.
    next.velocity[0] = wish[0] * speed;
    next.velocity[2] = wish[1] * speed;

    // Vertical.
    if next.in_water {
        // Buoyant, slow sinking, swim up with jump.
        let target = if input.jump { 3.0 } else { -1.6 };
        next.velocity[1] += (target - next.velocity[1]) * 0.25;
    } else {
        if input.jump && next.on_ground {
            next.velocity[1] = params.jump_speed;
            next.on_ground = false;
        }
        next.velocity[1] -= params.gravity * params.dt;
        next.velocity[1] = next.velocity[1].max(-params.terminal_speed);
    }

    // Integrate with collision, axis by axis.
    let mut position = next.position;
    let mut on_ground = false;

    // Y first so stepping onto a floor is handled before horizontal movement.
    let dy = next.velocity[1] * params.dt;
    let (new_y, hit_y) = sweep_axis(position, 1, dy, params, &is_solid);
    if hit_y {
        if dy < 0.0 {
            on_ground = true;
        }
        next.velocity[1] = 0.0;
    }
    position[1] = new_y;

    let dx = next.velocity[0] * params.dt;
    let (new_x, hit_x) = sweep_axis(position, 0, dx, params, &is_solid);
    position[0] = new_x;
    if hit_x {
        next.velocity[0] = 0.0;
    }

    let dz = next.velocity[2] * params.dt;
    let (new_z, hit_z) = sweep_axis(position, 2, dz, params, &is_solid);
    position[2] = new_z;
    if hit_z {
        next.velocity[2] = 0.0;
    }

    // Auto step-up: if we were blocked horizontally but there is room one step higher.
    if (hit_x || hit_z) && !next.in_water && params.step_height > 0.0 && (on_ground || next.on_ground) {
        let lift = params.step_height;
        let candidate = [position[0], position[1] + lift, position[2]];
        if !overlaps_solid(candidate, params, &is_solid) {
            let (landed_y, landed_hit) = sweep_axis(candidate, 1, -lift, params, &is_solid);
            if landed_hit {
                let landed = [position[0], landed_y, position[2]];
                if !overlaps_solid(landed, params, &is_solid) {
                    position = landed;
                    on_ground = true;
                }
            }
        }
    }

    // The world's bedrock floor and walls are solid blocks, so collision keeps the player in
    // bounds; only guard against falling through the bottom in a corrupt state.
    position[1] = position[1].max(0.0);

    next.position = position;
    next.on_ground = on_ground;
    next
}

/// Moves the player along one axis until it hits something, returning the new coordinate and
/// whether a collision happened.
fn sweep_axis(
    position: [f32; 3],
    axis: usize,
    delta: f32,
    params: &MoveParams,
    is_solid: &impl Fn(BlockPos) -> bool,
) -> (f32, bool) {
    if delta == 0.0 {
        return (position[axis], false);
    }
    // Sub-step so we never tunnel through a block.
    let steps = (delta.abs() / (params.radius * 0.5)).ceil().max(1.0) as u32;
    let step = delta / steps as f32;
    let mut value = position[axis];
    for _ in 0..steps {
        let mut probe = position;
        probe[axis] = value + step;
        if overlaps_solid(probe, params, is_solid) {
            return (value, true);
        }
        value = probe[axis];
    }
    (value, false)
}

/// Whether the player's AABB at `feet` overlaps any solid block.
pub fn overlaps_solid(
    feet: [f32; 3],
    params: &MoveParams,
    is_solid: &impl Fn(BlockPos) -> bool,
) -> bool {
    let min_x = feet[0] - params.radius;
    let max_x = feet[0] + params.radius;
    let min_y = feet[1];
    let max_y = feet[1] + params.height;
    let min_z = feet[2] - params.radius;
    let max_z = feet[2] + params.radius;

    let x0 = (min_x - 1e-4).floor() as i32;
    let x1 = (max_x + 1e-4).floor() as i32;
    let y0 = (min_y - 1e-4).floor() as i32;
    let y1 = (max_y + 1e-4).floor() as i32;
    let z0 = (min_z - 1e-4).floor() as i32;
    let z1 = (max_z + 1e-4).floor() as i32;
    for y in y0..=y1 {
        for z in z0..=z1 {
            for x in x0..=x1 {
                if is_solid(BlockPos::new(x, y, z)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Rotates a local input direction (x = strafe, y = forward) by a yaw angle into world space.
pub fn rotate_input(x: f32, forward: f32, yaw: f32) -> [f32; 2] {
    let sin = yaw.sin();
    let cos = yaw.cos();
    // Forward is -Z when yaw is zero.
    [x * cos - forward * sin, -x * sin - forward * cos]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_world(_: BlockPos) -> bool {
        false
    }

    fn floor_world(p: BlockPos) -> bool {
        p.y < 0
    }

    #[test]
    fn gravity_pulls_player_down_until_floor() {
        let params = MoveParams::default();
        let mut state = MovementState::at_spawn([10.5, 10.0, 10.5]);
        let input = MoveInput::idle(1, 0.0, 0.0);
        for seq in 2..200 {
            let mut i = input;
            i.seq = seq;
            state = step(&state, &i, &params, floor_world, |_| Medium::Air);
        }
        assert!(state.on_ground, "player should land");
        assert!(
            state.position[1].abs() < 0.05,
            "y = {}",
            state.position[1]
        );
    }

    #[test]
    fn walk_moves_at_walk_speed() {
        let params = MoveParams::default();
        let mut state = MovementState::at_spawn([10.5, 0.0, 10.5]);
        state.on_ground = true;
        let input = MoveInput {
            seq: 1,
            dir: [0.0, 1.0],
            jump: false,
            sprint: false,
            crouch: false,
            yaw: 0.0,
            pitch: 0.0,
        };
        for seq in 2..32 {
            let mut i = input;
            i.seq = seq;
            state = step(&state, &i, &params, flat_world, |_| Medium::Air);
        }
        // 30 ticks at 4.5 blocks/s along +Z ≈ 4.5 blocks.
        let travelled = state.position[2] - 10.5;
        assert!(
            (travelled - 4.5).abs() < 0.5,
            "travelled {travelled} blocks in one second"
        );
    }

    #[test]
    fn walls_block_movement() {
        let params = MoveParams::default();
        let wall = |p: BlockPos| p.x >= 12;
        let mut state = MovementState::at_spawn([10.5, 0.0, 10.5]);
        state.on_ground = true;
        for seq in 1..60 {
            let input = MoveInput {
                seq,
                dir: [1.0, 0.0],
                jump: false,
                sprint: false,
                crouch: false,
                yaw: 0.0,
                pitch: 0.0,
            };
            state = step(&state, &input, &params, wall, |_| Medium::Air);
        }
        assert!(state.position[0] < 12.0, "x = {}", state.position[0]);
    }

    #[test]
    fn prediction_is_deterministic() {
        let params = MoveParams::default();
        let mut a = MovementState::at_spawn([10.5, 5.0, 10.5]);
        let mut b = a;
        for seq in 1..50 {
            let input = MoveInput {
                seq,
                dir: [0.3, 0.9],
                jump: seq % 10 == 0,
                sprint: true,
                crouch: false,
                yaw: 0.7,
                pitch: 0.0,
            };
            a = step(&a, &input, &params, floor_world, |_| Medium::Air);
            b = step(&b, &input, &params, floor_world, |_| Medium::Air);
        }
        assert_eq!(a.position, b.position);
    }
}
