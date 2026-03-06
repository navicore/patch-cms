# Overview

The `cms-spool` crate implements the VM/CMS virtual spool subsystem. It models
the unit-record device queues -- reader, punch, and printer -- that CMS virtual
machines use to exchange files and produce printed output.

## Spool Concepts

### Devices

Three virtual unit-record devices are supported, mirroring the IBM VM/CMS
model:

| Device    | Aliases       | Purpose                              |
|-----------|---------------|--------------------------------------|
| Reader    | RDR, R        | Incoming files from other users      |
| Printer   | PRT, PR       | Output destined for printing         |
| Punch     | PUN, PCH, PU  | Card-punch output                    |

### Classes

Each device has an associated spool class (a single uppercase letter, A-Z).
Classes control routing and filtering -- for example, `QUERY READER CLASS N`
lists only class-N reader files.

### Spool IDs

Every file placed in a spool queue is assigned a unique numeric spool ID. The
ID is used by PURGE to target individual files for removal.

## Commands

All commands use CMS-style abbreviation matching (minimum unique prefix).

| Command   | Min Abbrev | Example                            | Description                          |
|-----------|------------|------------------------------------|--------------------------------------|
| SPOOL     | SP         | `SP PRT CLASS A DEST OPERATOR`     | Configure a device's class, destination, hold, continuous, or copy count |
| SENDFILE  | SE         | `SE MYFILE DATA A TO JONES`        | Send a file to another user's reader |
| RECEIVE   | REC        | `REC NEWNAME DATA A`               | Dequeue the next file from your reader |
| QUERY     | Q          | `Q READER`                         | List files waiting in a device queue |
| PURGE     | PUR        | `PUR READER 12345` / `PUR RDR ALL` | Remove one or all files from a queue |

These are represented in code by the `SpoolCommand` enum in `command.rs`, which
is parsed from raw token vectors via `SpoolCommand::parse()`.

## SpoolManager and the Backend Trait

`SpoolManager<B>` is the main entry point for spool operations. It is generic
over the `SpoolBackend` trait, which abstracts the storage layer:

```text
pub trait SpoolBackend {
    fn enqueue(...) -> Result<u64>;
    fn dequeue(device) -> Result<(SpoolFile, String)>;
    fn list_queue(device, class) -> Result<Vec<SpoolFile>>;
    ...
}
```

Two backend implementations are provided:

- **InMemoryBackend** -- hash-map-based storage used in tests.
- **DirectoryBackend** -- filesystem-backed storage using per-device
  subdirectories (`rdr/`, `prt/`, `pun/`) with `.meta` sidecar files.

`SpoolManager` owns per-device `DeviceConfig` structs that track the current
class, destination, hold, and copy-count settings. The SPOOL command modifies
these configs; SENDFILE and RECEIVE use them when enqueuing or dequeuing files.

`SpoolCommandResult` carries a return code and message list back to the caller,
following CMS conventions (rc 0 = success).

## Integration with CMS

The spool subsystem plugs into `cms-core`'s `CommandProcessor` through the
`ExtCommandHandler` trait. When the command processor encounters an unrecognized
command, it delegates to registered external handlers. The spool handler matches
SPOOL, SENDFILE, RECEIVE, QUERY, and PURGE, parses them into `SpoolCommand`
variants, and executes them against the `SpoolManager`. This keeps the spool
logic decoupled from the core CMS command loop while still appearing as
first-class CMS commands to the user.
