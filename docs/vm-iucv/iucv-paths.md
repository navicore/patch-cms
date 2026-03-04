# IUCV Paths

IUCV (Inter-User Communication Vehicle) paths provide bidirectional binary
data channels between two machines, modeled after the VM/CMS IUCV facility.

## Path Lifecycle

```
  connect()              on_connection_pending()
  (initiator)            (target decides: accept/refuse)
      │                         │
      │         ┌───────────────┤
      │         │ accept        │ refuse
      ▼         ▼               ▼
  ESTABLISHED           ConnectionRefused
      │
      │  ◄── iucv_send() / on_iucv_data()
      │
  sever() or logoff
      │
      ▼
  PATH REMOVED
```

## Establishing a Path

Use `supervisor.connect()` to request a path:

```rust
// Initiator requests a path to the target.
let path: PathId = supervisor.connect(&initiator, &target).await?;
```

The target's `on_connection_pending` is called. If it returns `true`, the path
is established and both sides receive `on_connection_complete`. If `false`,
the caller gets `ConnectionRefused`.

## Accepting/Refusing Connections

Override `on_connection_pending` to implement connection gating:

```rust
fn on_connection_pending(
    &mut self,
    _ctx: &MachineContext,
    _path: PathId,
    from: &MachineId,
) -> bool {
    // Only accept connections from TRUSTED
    from.as_str() == "TRUSTED"
}
```

The default implementation accepts all connections.

See the [connection_gating example](../EXAMPLES.md#connection_gating) for a
complete working version.

## Sending Data

Once a path is established, send binary data with `ctx.iucv_send()`:

```rust
use vm_iucv::path::IucvBuffer;

let data = IucvBuffer::new(b"Hello via IUCV".to_vec())?;
ctx.iucv_send(path, data)?;
```

The peer receives the data in `on_iucv_data`:

```rust
fn on_iucv_data(&mut self, _ctx: &MachineContext, path: PathId, data: IucvBuffer) {
    let text = String::from_utf8_lossy(data.as_bytes());
    println!("Received on {}: {}", path, text);
}
```

### IucvBuffer Limits

| Constraint | Limit |
|---|---|
| Max buffer size | 65,535 bytes |

## Severing a Path

Either side can sever a path:

```rust
// From outside a handler (via Supervisor)
supervisor.sever(path, &machine_id).await?;

// From inside a handler (via MachineContext)
ctx.sever_path(path)?;
```

The peer receives `on_connection_severed`. When a machine logs off, all its
paths are automatically severed and peers are notified.

## SMSG vs IUCV

| Feature | SMSG | IUCV |
|---|---|---|
| Data format | Text (ASCII, 236 bytes max) | Binary (65KB max) |
| Connection | None (fire-and-forget) | Explicit path lifecycle |
| Direction | One-way per message | Bidirectional on path |
| Delivery | Best-effort | Best-effort |
| Use case | Commands, notifications | Data transfer, sessions |
