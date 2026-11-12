# PROGRESS

Living record of what exists, what is deferred and how to run everything. A future session should
be able to continue from this file alone.

**Last updated:** milestone M2 complete (multiplayer foundation), M1 largely complete.

## How to run

```bash
cargo build                                    # debug (opt-level 1 for our code, 3 for deps)
cargo run -p deepcore_server -- --bind 127.0.0.1:5000 --seed 7
cargo run -p deepcore_client -- --connect 127.0.0.1:5000 --name Miner
```

Verification helpers:

```bash
scripts/check.sh          # fmt + clippy -D warnings + tests
scripts/smoke_test.sh     # real server + real client headless, prints both logs
scripts/screenshot.sh     # captures a PNG of a running session
cargo run -p deepcore_shared --bin balance_report
cargo run -p atlas_gen -- assets/data assets/generated/atlas.png
```

## Pinned versions (verified against crates.io and docs.rs, not memory)

| Crate | Version | Notes |
|---|---|---|
| bevy | 0.20.1 | pinned with `=` |
| bevy_replicon | 0.45.5 | message-based API (`add_client_message`), `ConnectedClient`, `ClientState` |
| bevy_replicon_renet | 0.21.0 | renet 2.0 / netcode 2.0 transport |
| bevy_egui | 0.43.1 | requires `render`, `bevy_ui`, `default_fonts` features |
| rusqlite | — | **dropped**: it conflicted with sqlx over the `links = "sqlite3"` symbol. Persistence uses region files + a RON world record instead. |

Bevy 0.20 API notes that cost time and are worth remembering:

* `CursorOptions` is a separate component; `Window` no longer has `cursor_options`.
* `DirectionalLight` has no `shadows_enabled` field.
* egui 0.36 panels (`CentralPanel`, `Panel`) take a `&mut Ui`, not a `&Context`; windows take a
  context. We use windows for every panel.
* `WindowResolution` is built from `(u32, u32)`.
* `bevy_egui` panics with "Dropped TexturesDelta" unless the `render` feature is on.
* The client and the server must create their netcode transports with the **same time base**
  (`Duration::ZERO`); otherwise the server rejects every packet as "connection expired".
* Replication rules and message registration must happen in **exactly** the same order on both
  sides. This is why `shared::protocol::register_protocol` is the only place that registers them,
  called once from `ProtocolPlugin` after `RepliconPlugins`.

## Milestone status

### M0 — workspace and skeleton: **done**
* Four crates (`shared`, `server`, `client`, `lobby`) plus `tools/atlas_gen`, edition 2024.
* Pinned versions, build profiles, `rustfmt.toml`, `clippy.toml`, `scripts/check.sh`.
* Server runs headless at 30 Hz (`Time::<Fixed>::from_hz(30.0)`), logs its seed and save dir.
* Client opens a window with a 3D camera, directional + ambient light.

### M1 — voxel world: **done**
* `Chunk` with palette compression (4-bit indices, 16-entry palette, raw `u16` fallback),
  `ChunkPos`/`BlockPos` maths, unit tests.
* Deterministic worldgen from `layers.ron`: six layers, noise tunnels and caverns, ore veins
  clustered in 4–40 block blobs, trees, bedrock floor and walls. Verified by a determinism test
  that compares every block of two independently generated chunks.
* Procedural texture atlas painted in memory at client startup (and by `tools/atlas_gen` as PNG).
* Greedy chunk meshing with cross-chunk face culling, run on `AsyncComputeTaskPool` at four jobs
  per frame. **3.67 ms per chunk** measured by `cargo run --example bench`.
* First-person controller with AABB-vs-voxel collision in `shared::movement`, shared with the
  server; block targeting raycast (Amanatides–Woo DDA); mining with hardness and tier rules;
  block placing; hotbar; depth meter.
* **Verified:** digging from the surface is implemented; the tier gate returns
  "Too hard — needs Tier N" and paints the crosshair red.

### M2 — multiplayer foundation: **done**
* Authoritative server + client over renet/UDP with a handshake that checks `PROTOCOL_VERSION`
  and the content hash (rejects with "Content mismatch").
* Chunk streaming with per-client interest (6 chunks horizontally, 3 vertically), nearest-first,
  zstd-compressed palette chunks, unload messages when the player leaves.
* Batched block-change messages per tick.
* Replicated players, movement prediction with rewind-and-replay reconciliation and 100 ms
  smoothing for corrections under 0.3 blocks.
* Mining and placing through intents with server-side reach, line-of-sight, tier and slot checks.
* Main menu with "Join by IP"; `--simulate-latency` / `--simulate-loss` flags on both binaries;
  `--bots N` on the server.
* **Verified:** server + client on one machine, client connects, receives the handshake, streams
  chunks and renders the terrain (see `scripts/screenshot.sh`).

### M3 — content, inventory, crafting (Era 0): **partly done**
* Complete RON content set: 93 blocks, 166 items, 113 recipes, 37 machines, 21 tech nodes,
  6 layers. Validator enforces every rule in the prompt.
* Inventory with stacking, split, quick-move, drag in egui, hotbar + 36-slot backpack, `proptest`
  conservation tests.
* Workbench hand crafting, death crate **deferred**, oil lamps and darkness **partly**.

### M4–M8 — machines, Eras 1–5: **systems done, wiring partly done**
* `MachineState` state machine with idle reasons ("No power", "Output full", "Missing inputs",
  "Requires research", "Cascade too short"), fuel burning, upgrade modules, output-space checks.
* Fuel, steam and electric networks with brownout scaling and battery buffers (`shared::power`,
  unit tested).
* Reactor tick with temperature, rods, scram and alarm.
* Centrifuge cascade length scaling.
* Nuclear chain present end to end in data (`crush → leach → precipitate → convert → enrich →
  fabricate → drill`), plus the reactor, cooling pool and waste vault.
* Machine placement creates a replicated block entity with a footprint check.
* Conveyor logistics: machines push outputs into the machine in front of them.

### M9 — persistence: **partly done**
* Region files (8×8×8 chunks) with `postcard` + atomic rename, and a `world.ron` record holding
  edits, machines, research and players.
* Autosave every 5 minutes, serialised on the main thread and written on a background task.
* **Not yet wired:** loading a save back into the world on startup.

### M10 — lobby: **service done, client UI deferred**
* `axum` + `sqlx` (SQLite) service with argon2 passwords, session tokens, rate-limited login,
  server registration, heartbeat and browser listing. Unit tests for hashing and tokens.
* Client-side login/browser screens and Dockerfiles **deferred**.

## Known issues

1. **World generation throughput.** The initial view (≈845 chunks) takes ~25 s to generate on two
   cores; the client shows the sky until chunks arrive. Mitigations in place: 256 chunks polled
   per tick, 24 chunks streamed per tick while the backlog is large, vertical radius reduced to 3.
   A proper fix is a column-cached generator shared between neighbouring chunks.
2. **Machines do not yet consume from the world automatically.** Players can place machines, set
   recipes and move items, but the recipe *selection* UI only lists recipes and the transfer
   buttons are minimal. Automation (conveyors) is implemented at the data level.
3. **Client machine UI opening** relies on the server sending `MachineContents` after an
   `Interact`; the panel opens but the transfer buttons are not yet wired to `MachineTransfer`.
4. **Player models are not drawn** for other players; only their replicated transforms exist.
   A simple capsule view model is deferred.
5. **Audio is silent in headless/CI environments** (no device); the systems degrade gracefully.
6. **Bots** are spawned as marker entities but do not yet issue intents.
7. **Save loading on startup** is not implemented (writing works).
8. **`cargo clippy -D warnings` is not yet clean** — there are unused-import and
   needless-mut warnings across the server crate.

## Exact commands for the next session

```bash
# Everything that must be green
scripts/check.sh

# Manual smoke test
./scripts/smoke_test.sh 5090 40

# Inspect content after an edit
cargo run -p deepcore_shared --bin balance_report
cargo test -p deepcore_shared
```

## File map

```
crates/shared/src/
  content/{defs,ids,registry}.rs   RON shapes, numeric ids, loader + validator
  voxel/{chunk,codec,raycast}.rs   palette chunks, wire codec, DDA raycast
  worldgen/generator.rs            layers, caves, veins, trees, pockets
  movement.rs                      AABB physics, shared by both sides
  inventory.rs                     stacks, slots, transfers, proptest
  crafting.rs                      recipe matching, machine state machine
  power.rs                         steam + electric networks
  hazards.rs                       vitals, dose, heat, flooding, cave-ins
  protocol.rs                      PROTOCOL_VERSION, components, intents, messages
  bin/balance_report.rs            pacing report

crates/server/src/
  main.rs                          clap args, plugin wiring
  net/{transport,handshake,stream}.rs
  world/{mod,persistence}.rs
  sim/{mod,components,intents,mining,machines,power,hazards,movement,logistics}.rs

crates/client/src/
  main.rs                          window, plugins, args
  render/mod.rs                    atlas, greedy mesher, chunk entities
  world.rs                         client chunk store
  input/mod.rs                     mouse look, movement, targeting
  prediction/mod.rs                predict + reconcile
  net/mod.rs                       connect, handshake, message handling
  hud/mod.rs                       crosshair, vitals, depth, Geiger, debug
  ui/mod.rs                        menu, inventory, machine, tech tree, overlay
  audio/mod.rs                     procedural tones

crates/lobby/src/main.rs           axum service
tools/atlas_gen/src/main.rs        PNG atlas generator
assets/data/*.ron                  all game content
```
