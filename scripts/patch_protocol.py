import re

# --- shared/src/protocol.rs: single registration entry point -----------------
p = 'crates/shared/src/protocol.rs'
s = open(p).read()
old_start = s.index('/// Registers every replicated component, in a fixed order')
old_end = s.index('// ---------------------------------------------------------------------------\n// Client → server intents')
new_block = '''/// Registers every replicated component and every network message, in a fixed order that must be
/// identical on the client and the server.
///
/// This is the single place the protocol is described; both binaries call it exactly once, from
/// `SharedPlugin`.
pub fn register_protocol(app: &mut bevy::prelude::App) {
    use bevy_replicon::prelude::AppRuleExt;

    // Replication rules first: their order is part of replicon's protocol hash.
    app.replicate::<NetPlayer>()
        .replicate::<NetMovement>()
        .replicate::<NetHealth>()
        .replicate::<NetHeld>()
        .replicate::<NetMachine>()
        .replicate::<NetPickup>()
        .replicate::<NetLight>();

    // Client -> server.
    app.add_client_message::<ClientHello>(Channel::Ordered)
        .add_client_message::<Intent>(Channel::Ordered);

    // Server -> client. The order of these calls assigns channel IDs, so it must match exactly.
    app.add_server_message::<ServerInfo>(Channel::Ordered)
        .add_server_message::<HandshakeRejected>(Channel::Ordered)
        .add_server_message::<ChunkData>(Channel::Ordered)
        .add_server_message::<ChunkUnload>(Channel::Ordered)
        .add_server_message::<BlockChanges>(Channel::Ordered)
        .add_server_message::<InventoryUpdate>(Channel::Ordered)
        .add_server_message::<MachineContents>(Channel::Ordered)
        .add_server_message::<TechState>(Channel::Ordered)
        .add_server_message::<ChatLine>(Channel::Ordered)
        .add_server_message::<Notify>(Channel::Ordered)
        .add_server_message::<ScannerResult>(Channel::Ordered)
        .add_server_message::<MovementAck>(Channel::Unordered)
        .add_server_message::<NetworkSnapshot>(Channel::Unordered);
}

'''
s = s[:old_start] + new_block + s[old_end:]
s = s.replace('use bevy::prelude::*;', 'use bevy::prelude::*;\nuse bevy_replicon::prelude::Channel;', 1)
open(p, 'w').write(s)

# --- shared/src/lib.rs: call it from SharedPlugin ---------------------------
p = 'crates/shared/src/lib.rs'
s = open(p).read()
s = s.replace('''            .add_message::<protocol::ClientHello>();''','''            .add_message::<protocol::ClientHello>();

        // Every replicated component and network message is registered here so the client and the
        // server can never drift apart.
        protocol::register_protocol(app);''')
open(p, 'w').write(s)

# --- server: drop its own registration --------------------------------------
p = 'crates/server/src/main.rs'
s = open(p).read()
s = s.replace('    net::transport::register_messages(&mut app);\n', '')
s = s.replace('    sim::register_replication(&mut app);\n', '')
open(p, 'w').write(s)

p = 'crates/server/src/sim/mod.rs'
s = open(p).read()
s = s.replace('''/// Registers replication rules for every replicated component.
pub fn register_replication(app: &mut App) {
    deepcore_shared::protocol::register_replication(app);
}

''', '')
s = s.replace('use deepcore_shared::protocol::{\n    InventoryUpdate, NetHealth, NetHeld, NetLight, NetMachine, NetMovement, NetPickup, NetPlayer,\n    TechState,\n};', 'use deepcore_shared::protocol::{InventoryUpdate, TechState};')
open(p, 'w').write(s)

# --- client: drop its own registration --------------------------------------
p = 'crates/client/src/main.rs'
s = open(p).read()
s = s.replace('    net::register_messages(&mut app);\n', '')
open(p, 'w').write(s)

p = 'crates/client/src/net/mod.rs'
s = open(p).read()
start = s.index('/// Registers every message type used over the wire, in the same order as the server.')
end = s.index('/// The client\'s network state.')
s = s[:start] + s[end:]
open(p, 'w').write(s)

print("patched")
