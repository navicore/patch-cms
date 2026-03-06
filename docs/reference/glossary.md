# VM/CMS Glossary

A mapping of VM/CMS terminology to Rust concepts in this workspace.

## VM & Supervisor

| VM/CMS Term | Rust Concept | Description |
|---|---|---|
| **CP** (Control Program) | `Supervisor` | The hypervisor that manages virtual machines |
| **Virtual Machine** | `MachineId` + `MachineHandler` | An isolated execution context with its own handler |
| **IPL** (Initial Program Load) | `supervisor.ipl()` | Boot a virtual machine |
| **LOGOFF** | `supervisor.logoff()` | Shut down a virtual machine |

## Messaging & Communication

| VM/CMS Term | Rust Concept | Description |
|---|---|---|
| **CP SMSG** | `supervisor.smsg()` / `ctx.try_send_smsg()` | Fire-and-forget text message (max 236 bytes) |
| **IUCV** (Inter-User Communication Vehicle) | Path API (`connect`, `sever`, `iucv_send`) | Bidirectional data channel between machines |
| **IUCV CONNECT** | `supervisor.connect()` | Request a path to another machine |
| **IUCV ACCEPT** | `on_connection_pending` returning `true` | Accept an incoming connection request |
| **IUCV SEVER** | `supervisor.sever()` / `ctx.sever_path()` | Tear down an established path |
| **IUCV SEND** | `ctx.iucv_send()` | Send binary data on a path |
| **IUCV RECEIVE** | `on_iucv_data()` callback | Receive binary data on a path |
| **Path** | `PathId` | Identifier for an IUCV connection |

## CMS File System

| VM/CMS Term | Rust Concept | Description |
|---|---|---|
| **fn ft fm** | `FileSpec` | CMS file naming: filename, filetype, filemode (e.g., `PROFILE EXEC A`) |
| **Minidisk** | `AccessMode` + disk path | Virtual disk accessed at a filemode letter (A-Z) |
| **Access** | `CmsFileSystem::access_disk()` | Mount a disk directory at a filemode letter |
| **LISTFILE** | `cmd_listfile()` | List files matching a pattern |
| **COPYFILE** | `cmd_copyfile()` | Copy files between disks |

## Command Processor

| VM/CMS Term | Rust Concept | Description |
|---|---|---|
| **CMS command** | `CommandProcessor::execute()` | Dispatch a command through the processor |
| **Abbreviation** | `parse_cms_command()` | IBM-style minimum abbreviation matching (e.g., `GLO` for `GLOBALV`) |
| **EXEC** | `ExecHandler::execute_exec()` | Run a REXX or EXEC2 program |
| **GLOBALV** | `GlobalVars` | Persistent named variables (SET/GET/LIST, LASTING/SESSION groups) |
| **ExtCommandHandler** | `ExtCommandHandler` trait | Extension point for spool/pipeline commands |

## Spool Subsystem

| VM/CMS Term | Rust Concept | Description |
|---|---|---|
| **Spool** | `SpoolManager` | Virtual I/O queue manager for reader/punch/printer |
| **Reader (RDR)** | Reader queue | Inbound spool device for receiving files |
| **Punch (PUN)** | Punch queue | Outbound spool device for sending files |
| **Printer (PRT)** | Printer queue | Output spool device |
| **Spool class** | `SpoolClass` | Single-letter classification (A-Z, 0-9, *) |
| **Spool ID** | `SpoolId` | Numeric identifier for a spooled file |
| **SENDFILE** | `execute_spool_command(SendFile)` | Send a file to another user's reader |
| **RECEIVE** | `execute_spool_command(Receive)` | Receive a file from the reader queue |

## Pipelines

| VM/CMS Term | Rust Concept | Description |
|---|---|---|
| **PIPE** | `run_pipe()` | Execute a Hartmann pipeline |
| **Stage** | `Stage` trait | A processing unit in a pipeline |
| **literal** | Built-in stage | Emits a literal string |
| **console** | Built-in stage | Writes records to output (like terminal display) |
| **locate** | Built-in stage | Passes records matching a pattern |
| **nlocate** | Built-in stage | Passes records NOT matching a pattern |

## XEDIT Editor

| VM/CMS Term | Rust Concept | Description |
|---|---|---|
| **XEDIT** | `xedit-core` | Full-screen editor |
| **Prefix area** | Prefix command model | Left margin for line commands (d, dd, i, a, c, m, etc.) |
| **Current line** | `Editor::current_line()` | The line the cursor is positioned on |
| **Target** | Target system | Navigation spec (`:n`, `/string/`, `*`, compound) |
| **EXTRACT** | REXX extract variables | Query editor state from macros |
| **PROFILE XEDIT** | Profile exec | Startup macro executed when editor opens |
| **Command line** | `====>` prompt | Where editor commands are entered |

## General

| VM/CMS Term | Rust Concept | Description |
|---|---|---|
| **EBCDIC** | ASCII (simplified) | Character encoding for messages |
| **DMSIUC** | `IucvError` display prefix | IBM message prefix for IUCV errors |
| **RC** (Return Code) | `rc()` methods | Numeric error code (CMS convention: 0=ok, 24=error, 28=not found) |
| **Signal** | Internal `MachineSignal` enum | Event dispatched to a machine's handler |
| **Console** | `CollectorHandler` | Test utility that captures messages (like a virtual console) |
| **QUERY NAMES** | `supervisor.query_names()` | List all running machines |
