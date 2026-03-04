# API Quick Reference

## Supervisor

| Method | Description |
|---|---|
| `Supervisor::new()` | Create a new supervisor (must be called in a Tokio runtime) |
| `ipl(&id, handler)` | Boot a machine with the given handler |
| `logoff(&id)` | Log off a running machine |
| `smsg(&from, &to, text)` | Send an SMSG between machines |
| `connect(&from, &to)` | Establish an IUCV path between machines |
| `sever(path, &from)` | Sever an IUCV path |
| `query_names()` | List all running machine IDs |
| `query_paths()` | List all active path IDs |
| `shutdown()` | Log off all machines and stop the supervisor |

All `Supervisor` methods are `async` except `new()`.

## MachineContext

| Method | Description |
|---|---|
| `machine_id()` | Get this machine's `MachineId` |
| `try_send_smsg(&to, text)` | Send an SMSG (non-blocking, fire-and-forget) |
| `iucv_send(path, buffer)` | Send data on an IUCV path (non-blocking) |
| `sever_path(path)` | Sever an IUCV path (non-blocking) |

All `MachineContext` methods are synchronous (non-async) and use `try_send`.

## MachineHandler Trait

| Callback | Signature | Default |
|---|---|---|
| `on_ipl` | `(&mut self, &MachineContext)` | No-op |
| `on_smsg` | `(&mut self, &MachineContext, SmsgMessage)` | **Required** |
| `on_connection_pending` | `(&mut self, &MachineContext, PathId, &MachineId) -> bool` | `true` |
| `on_connection_complete` | `(&mut self, &MachineContext, PathId, &MachineId)` | No-op |
| `on_connection_severed` | `(&mut self, &MachineContext, PathId, &MachineId)` | No-op |
| `on_iucv_data` | `(&mut self, &MachineContext, PathId, IucvBuffer)` | No-op |
| `on_logoff` | `(&mut self, &MachineContext)` | No-op |

## Key Types

| Type | Description |
|---|---|
| `MachineId` | Machine identity (1-8 chars, auto-uppercased) |
| `SmsgMessage` | Text message (ASCII, max 236 bytes) |
| `PathId` | Opaque IUCV path identifier (Copy, Eq, Hash) |
| `IucvBuffer` | Binary data buffer (max 65,535 bytes) |
| `IucvError` | Error type with CMS-style return codes |

## CollectorHandler (test-util feature)

| Function/Method | Description |
|---|---|
| `collector()` | Create a `(CollectorHandler, CollectorHandle)` pair |
| `handle.messages()` | Snapshot of collected SMSG messages |
| `handle.count()` | Number of collected messages |
| `handle.path_events()` | Snapshot of collected path events |
| `handle.path_event_count()` | Number of collected path events |
