# Deepcore — Implementation Plan

## What we are building

Deepcore is a server-authoritative, online co-op, first-person voxel mining game written entirely in
Rust on Bevy. 2–16 players share one persistent 1024 × 1024 × 512 block mine. Everyone starts with a
steel pickaxe. The loop is: **dig deeper → haul ore home → build a machine → unlock a better tool →
dig deeper**, repeated over six Eras until the team can refine uranium into a **Nuclear Drill** and run
a **reactor** without melting the Base.

The whole game is one Bevy ECS. The server runs the same components and systems as the client but
headless (`MinimalPlugins`) and is the only authority. The client renders, predicts only its own
movement, and sends intents.

## Crate layout

```
deepcore/
  crates/shared/   deepcore_shared  — every rule of the game, no sockets, no rendering
      content/     RON loaders, registries (BlockId, ItemId, RecipeId, MachineId, TechId), validation, hash
      voxel/       Chunk, ChunkPos, BlockPos, palette compression, voxel raycast
      worldgen/    layers, caves, ore veins, trees, deterministic from seed
      movement/    AABB-vs-voxel collision + player controller (pure, fixed timestep)
      inventory/   stacks, slots, transfer rules, hotbar/backpack
      crafting/    recipe matching, machine process state machine, idle reasons
      power/       steam + electricity network graphs and solvers
      hazards/     heat, gas, water, radiation, cave-in maths
      protocol/    PROTOCOL_VERSION, replicated components, client intents, server events, channels
      lib.rs       SharedPlugin
  crates/server/   deepcore_server — headless authoritative binary
      net/         renet transport, handshake, content-hash check, per-client visibility
      world/       chunk store, generation workers, persistence (region files + SQLite)
      sim/         mining validation, machine ticks, power solve, hazards, bots
  crates/client/   deepcore_client — windowed binary
      render/      chunk meshing on AsyncComputeTaskPool, atlas material, lighting, outline
      input/       first person controls, block targeting
      prediction/  movement prediction + reconciliation
      ui/          egui: inventory, crafting, machine UI, tech tree, network overlay
      hud/         hotbar, crosshair, vitals, depth meter, Geiger
      audio/       procedural tones for dig/place/machine/alarm/geiger
  crates/lobby/    deepcore_lobby — axum + sqlx accounts, tokens, server list (M10)
  tools/atlas_gen/ procedural block texture atlas generator (also a library used by the client)
  assets/data/     blocks, items, recipes, machines, tech_tree, layers, hazards (.ron)
```

Rule of thumb: **`shared` holds every rule, `server` decides, `client` shows.** Anything that decides
a game outcome lives in `shared` and is called by the server. The client only calls `shared` for
prediction and display.

## Technical decisions (verified against docs, not memory)

* Bevy **0.20.1**, `bevy_replicon` **0.45.5**, `bevy_replicon_renet` **0.21.0** (renet netcode/UDP),
  `bevy_egui` **0.43.1**. All four support each other; versions are pinned with `=`.
* Replicon 0.45 uses **messages** (`add_client_message`, `add_server_message`) rather than 0.3x-era
  events, `ConnectedClient` entities, `AppRuleExt::replicate`, `ClientState`/`ServerState` states.
  Our intents are `Message` types sent with `MessageWriter<Intent>` and read on the server as
  `MessageReader<FromClient<Intent>>`.
* Replication runs on the replication tick, which we set to 30 Hz (`Time::from_hz(30.0)` for `Fixed`)
  to match the simulation tick.
* Chunk data and block changes travel as our own server messages on dedicated channels, not as
  replicated components (they are bulk binary, not ECS data).
* Voxel meshing is our own greedy mesher in `shared::voxel::mesh` (pure data, unit-testable), run on
  `AsyncComputeTaskPool` in the client.
* Physics is our own AABB-vs-voxel sweep — deterministic, identical on both sides.
* Save format: region files (`postcard` + `zstd`) for modified chunks, SQLite for players/tech/machines.

## Milestones (from the prompt, in order)

| # | Milestone | Acceptance |
|---|---|---|
| M0 | Workspace + skeleton | `cargo run -p server` and `cargo run -p client` start; `scripts/check.sh` passes |
| M1 | Voxel world, single process | dig from surface through layer 2, blocked by granite at layer 3 with tier-1 pick; 60 FPS @ radius 6 |
| M2 | Multiplayer foundation | 2 clients see each other move/dig; responsive at 150 ms latency; 8 bots 10 min no desync |
| M3 | Content, inventory, crafting (Era 0) | Era 0 playable in MP, validator passes |
| M4 | Machines, research, Era 1 | team reaches Hardened Steel Pickaxe and layer 3 |
| M5 | Era 2: steam + logistics | automate ore → ingot, reach layer 4 |
| M6 | Era 3: electricity | power plant, brownouts, gas cleared |
| M7 | Era 4: chemistry | reach layer 5, find uranium, survive |
| M8 | Era 5: nuclear | headless integration test of the whole nuclear chain; 3×3 tunnel into layer 6 |
| M9 | Persistence + polish | restart restores everything; objectives guide new players |
| M10 | Lobby, accounts, browser | Docker lobby + dedicated server, two clients register and play |

## Working rules we hold ourselves to

* Compile after every meaningful change; `scripts/check.sh` (fmt + clippy `-D warnings` + tests)
  before closing a milestone.
* No `todo!()`, no empty systems, no fake features. Deferred work is listed in `PROGRESS.md`.
* All content in RON; the validator fails fast on dangling references, unusable recipes, orphan tech
  nodes and ores with no use.
* Files stay small and named by purpose; one plugin per feature.
* Every public type and plugin in `shared` carries a doc comment.
* `PROTOCOL_VERSION` bumps whenever replicated shapes or intents change.
