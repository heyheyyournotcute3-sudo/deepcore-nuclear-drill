//! Procedural audio: dig, place, machine hum, reactor alarm and Geiger clicks.
//!
//! Every sound is synthesised at startup into an `AudioSource`, so no audio files are shipped.

use bevy::audio::{AudioPlayer, AudioSink, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::hud::Geiger;
use crate::input::InputState;

/// Handle to every synthesised sound.
#[derive(Resource, Default)]
pub struct Sounds {
    /// Dig tick.
    pub dig: Option<Handle<AudioSource>>,
    /// Block break.
    pub break_block: Option<Handle<AudioSource>>,
    /// Block place.
    pub place: Option<Handle<AudioSource>>,
    /// Machine hum loop.
    pub machine: Option<Handle<AudioSource>>,
    /// Reactor alarm.
    pub alarm: Option<Handle<AudioSource>>,
    /// Geiger click.
    pub geiger: Option<Handle<AudioSource>>,
}

/// The audio plugin.
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sounds>()
            .add_systems(Startup, build_sounds)
            .add_systems(Update, (play_dig, play_geiger));
    }
}

const SAMPLE_RATE: u32 = 44_100;

/// Builds an `AudioSource` from mono f32 samples.
fn source(samples: Vec<f32>) -> AudioSource {
    let bytes: Vec<u8> = samples
        .iter()
        .flat_map(|s| s.to_le_bytes().to_vec())
        .collect();
    AudioSource { bytes: bytes.into() }
}

fn tone(freq: f32, seconds: f32, decay: f32, noise: f32) -> Vec<f32> {
    let count = (SAMPLE_RATE as f32 * seconds) as usize;
    let mut out = Vec::with_capacity(count);
    let mut rng: u32 = 0x1234_5678;
    for i in 0..count {
        let t = i as f32 / SAMPLE_RATE as f32;
        rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let white = ((rng >> 8) as f32 / 8_388_608.0) - 1.0;
        let envelope = (-t * decay).exp();
        let sample = ((t * freq * std::f32::consts::TAU).sin() * (1.0 - noise)
            + white * noise)
            * envelope
            * 0.35;
        out.push(sample);
    }
    out
}

fn build_sounds(mut commands: Commands, sources: Option<ResMut<Assets<AudioSource>>>) {
    let Some(mut sources) = sources else {
        // Audio is unavailable (headless or no audio device); the game runs silently.
        return;
    };
    let dig = sources.add(source(tone(180.0, 0.08, 40.0, 0.7)));
    let break_block = sources.add(source(tone(110.0, 0.22, 14.0, 0.6)));
    let place = sources.add(source(tone(320.0, 0.1, 30.0, 0.3)));
    let machine = sources.add(source(tone(70.0, 1.0, 0.4, 0.15)));
    let alarm = sources.add(source(tone(880.0, 0.5, 3.0, 0.05)));
    let geiger = sources.add(source(tone(2400.0, 0.012, 260.0, 0.9)));
    commands.insert_resource(Sounds {
        dig: Some(dig),
        break_block: Some(break_block),
        place: Some(place),
        machine: Some(machine),
        alarm: Some(alarm),
        geiger: Some(geiger),
    });
}

fn play_dig(
    mouse: Res<ButtonInput<MouseButton>>,
    input: Res<InputState>,
    sounds: Res<Sounds>,
    mut commands: Commands,
    mut timer: Local<f32>,
    time: Res<Time>,
) {
    if !input.captured || !mouse.pressed(MouseButton::Left) || input.target.is_none() {
        *timer = 0.0;
        return;
    }
    *timer += time.delta_secs();
    if *timer > 0.25 {
        *timer = 0.0;
        if let Some(handle) = sounds.dig.clone() {
            commands.spawn((
                AudioPlayer(handle),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.4)),
            ));
        }
    }
}

fn play_geiger(
    geiger: Res<Geiger>,
    sounds: Res<Sounds>,
    mut commands: Commands,
    time: Res<Time>,
    mut accumulator: Local<f32>,
) {
    if geiger.rate <= 0.0 {
        *accumulator = 0.0;
        return;
    }
    *accumulator += time.delta_secs() * geiger.rate;
    while *accumulator >= 1.0 {
        *accumulator -= 1.0;
        if let Some(handle) = sounds.geiger.clone() {
            commands.spawn((
                AudioPlayer(handle),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.25)),
            ));
        }
    }
}

/// Plays a one-shot sound effect.
pub fn play_once(commands: &mut Commands, handle: Option<Handle<AudioSource>>) {
    if let Some(handle) = handle {
        commands.spawn((
            AudioPlayer(handle),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.5)),
        ));
    }
}

/// A looping machine hum that follows the number of running machines.
pub fn spawn_machine_hum(commands: &mut Commands, sounds: &Sounds) -> Option<Entity> {
    sounds.machine.clone().map(|handle| {
        commands
            .spawn((
                AudioPlayer(handle),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(0.15)),
            ))
            .id()
    })
}

/// Stops a loop by despawning it.
pub fn stop_loop(commands: &mut Commands, entity: Option<Entity>) {
    if let Some(entity) = entity {
        commands.entity(entity).despawn();
    }
}

/// Keeps the audio sink type referenced so the import stays valid across Bevy versions.
#[allow(dead_code)]
fn _sink_type(_: &AudioSink) {}
