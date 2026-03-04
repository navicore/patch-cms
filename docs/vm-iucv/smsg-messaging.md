# SMSG Messaging

SMSG (Special Message) is the fire-and-forget text messaging facility, modeled
after the VM/CMS `CP SMSG` command.

## Sending Messages

There are two ways to send an SMSG:

### From outside a handler (via Supervisor)

```rust
// Async — awaits until the message is enqueued in the router
supervisor.smsg(&from, &to, "Hello").await?;
```

### From inside a handler (via MachineContext)

```rust
impl MachineHandler for MyHandler {
    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
        // Non-blocking — uses try_send
        let _ = ctx.try_send_smsg(msg.from(), "Got it!");
    }
}
```

## Message Constraints

| Constraint | Limit |
|---|---|
| Max text length | 236 bytes |
| Character set | ASCII only |

These match the real CP SMSG limits (236-byte EBCDIC text).

```rust
// Constructing a message manually
let msg = SmsgMessage::new(
    MachineId::new("ALICE").unwrap(),
    MachineId::new("BOB").unwrap(),
    "Hello Bob",
)?;
```

## Delivery Semantics

SMSG delivery is **best-effort**:

- If the target machine exists and its channel has room, the message is
  delivered to `on_smsg`.
- If the target's channel is full, the message is **silently dropped**.
- If the target is not logged on, `supervisor.smsg()` returns
  `MachineNotFound`.
- Messages in-flight when a machine logs off may or may not be delivered.

This matches real VM/CMS behavior where CP SMSG provides no delivery
guarantee.

## Request/Reply Pattern

Since SMSG is fire-and-forget, implement request/reply by having the receiver
send a response back:

```rust
struct EchoHandler;

impl MachineHandler for EchoHandler {
    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
        let reply = format!("ECHO: {}", msg.text());
        let _ = ctx.try_send_smsg(msg.from(), &reply);
    }
}
```

See the [echo_server example](../EXAMPLES.md#echo_server) for a complete
working version.

## Inspecting Messages (Testing)

Use the `CollectorHandler` (requires `test-util` feature) to capture messages
for inspection:

```rust
use vm_iucv::collector::collector;

let (handler, handle) = collector();
supervisor.ipl(&id, handler).await?;

// ... send messages ...

let messages = handle.messages();
assert_eq!(messages[0].text(), "Hello");
```
