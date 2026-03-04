# VM/CMS Glossary

A mapping of VM/CMS terminology to `vm-iucv` Rust concepts.

| VM/CMS Term | Rust Concept | Description |
|---|---|---|
| **CP** (Control Program) | `Supervisor` | The hypervisor that manages virtual machines |
| **Virtual Machine** | `MachineId` + `MachineHandler` | An isolated execution context with its own handler |
| **IPL** (Initial Program Load) | `supervisor.ipl()` | Boot a virtual machine |
| **LOGOFF** | `supervisor.logoff()` | Shut down a virtual machine |
| **CP SMSG** | `supervisor.smsg()` / `ctx.try_send_smsg()` | Fire-and-forget text message (max 236 bytes) |
| **IUCV** (Inter-User Communication Vehicle) | Path API (`connect`, `sever`, `iucv_send`) | Bidirectional data channel between machines |
| **IUCV CONNECT** | `supervisor.connect()` | Request a path to another machine |
| **IUCV ACCEPT** | `on_connection_pending` returning `true` | Accept an incoming connection request |
| **IUCV SEVER** | `supervisor.sever()` / `ctx.sever_path()` | Tear down an established path |
| **IUCV SEND** | `ctx.iucv_send()` | Send binary data on a path |
| **IUCV RECEIVE** | `on_iucv_data()` callback | Receive binary data on a path |
| **Path** | `PathId` | Identifier for an IUCV connection |
| **EBCDIC** | ASCII (simplified) | Character encoding for messages |
| **DMSIUC** | `IucvError` display prefix | IBM message prefix for IUCV errors |
| **RC** (Return Code) | `IucvError::rc()` | Numeric error code (CMS convention) |
| **Signal** | Internal `MachineSignal` enum | Event dispatched to a machine's handler |
| **Console** | `CollectorHandler` | Test utility that captures messages (like a virtual console) |
| **QUERY NAMES** | `supervisor.query_names()` | List all running machines |
