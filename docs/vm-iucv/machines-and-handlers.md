# Machines & Handlers

## MachineId

Every machine has a unique identity — a `MachineId`. IDs follow VM/CMS naming
rules:

- 1–8 characters
- Must start with a letter or national character (`@`, `#`, `$`)
- Remaining characters: alphanumeric or national
- Automatically uppercased

```rust
let id = MachineId::new("alice").unwrap(); // becomes "ALICE"
assert_eq!(id.as_str(), "ALICE");
```

## MachineHandler Trait

Every machine needs a handler that implements `MachineHandler`. The only
required method is `on_smsg`:

```rust
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::message::SmsgMessage;

struct MyHandler;

impl MachineHandler for MyHandler {
    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
        println!("Got: {}", msg.text());
    }
}
```

### Lifecycle Callbacks

| Callback | When Called | Required |
|---|---|---|
| `on_ipl` | After boot, before signals | No |
| `on_smsg` | For each incoming SMSG | **Yes** |
| `on_connection_pending` | Peer requests a path | No (default: accept) |
| `on_connection_complete` | Path fully established | No |
| `on_connection_severed` | Path severed by peer | No |
| `on_iucv_data` | Data arrives on a path | No |
| `on_logoff` | Machine logging off | No |

### Important Notes

- All callbacks are **synchronous** — they run on the machine's Tokio task.
  Blocking a callback stalls the entire machine.
- `on_logoff` is **not called** if `on_ipl` or `on_smsg` panics. Use RAII
  guards for guaranteed cleanup.
- `on_connection_complete` and `on_connection_severed` use best-effort
  delivery — they may be skipped if the signal channel is full.

## MachineContext

The `MachineContext` is passed to every callback and provides the machine's
identity and communication methods:

```rust
// Identity
ctx.machine_id()          // → &MachineId

// SMSG (fire-and-forget)
ctx.try_send_smsg(&to, "text")  // → Result<()>

// IUCV path operations
ctx.iucv_send(path, buffer)     // → Result<()>
ctx.sever_path(path)            // → Result<()>
```

All `MachineContext` methods are non-blocking — they use `try_send` internally
and return immediately.

## Machine Lifecycle

```
Supervisor::ipl()
    │
    ▼
on_ipl()          ← one-time initialization
    │
    ▼
┌─────────┐
│ Signal   │◄──── on_smsg(), on_connection_*, on_iucv_data()
│ Loop     │
└────┬────┘
     │
     ▼
on_logoff()       ← cleanup (best-effort)
     │
     ▼
  (task ends)
```
