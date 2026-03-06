# Overview

`cms-core` implements the CMS (Conversational Monitor System) layer of the
VM/CMS reimplementation. It provides the file system, command processor,
session variables (GLOBALV), and EXEC resolution -- the foundation that
higher-level crates (`cms-spool`, `cms-pipelines`, `cms-machine`) build on.

## Key Concepts

### CMS File System

Files are identified by a three-part name: **filename**, **filetype**, and
**filemode** (fn ft fm). The filemode letter (A-Z) selects a minidisk;
the wildcard `*` searches all accessed disks in alphabetical order.

`CmsFileSystem` manages a `BTreeMap<char, Minidisk>` and exposes CMS-style
operations: read, write, erase, rename, copy, list, and state (existence
check). Disks are accessed with a mode (read-only or read-write) via the
ACCESS command and released with RELEASE.

### CommandProcessor

The central dispatch engine. It owns the file system, GLOBALV store, an
`ExecHandler`, an `ExtCommandHandler`, and an optional `SmsgSender`. All
command execution flows through `CommandProcessor::execute(input)`.

### GLOBALV

`GlobalVars` provides session-scoped named variables organized into groups
(default group: `LASTING`). Group and variable names are uppercased; values
are stored as-is. Supports SELECT, SET, GET, LIST, DELETE, and PURGE
sub-commands, matching IBM CMS GLOBALV semantics.

### EXEC Resolution

When the command processor encounters an unknown command, it searches for a
file named `<COMMAND> EXEC *` across all accessed disks. If found, the file
is handed to the `ExecHandler` for interpretation (typically REXX via
`patch-rexx`).

## Trait Seams

The crate uses trait-based seams to stay decoupled from the REXX interpreter,
the actor framework, and extension command sets.

| Trait | Purpose | Default (no-op) |
|---|---|---|
| `ExecHandler` | Run REXX EXEC files; swap fs/gv/smsg state in and out | `NoExecHandler` (RC=28) |
| `SmsgSender` | Send SMSG messages to other virtual machines | `NoSmsgSender` (RC=28) |
| `ExtCommandHandler` | Handle commands outside cms-core (spool, pipelines) | `NoExtCommands` (None) |

`ExecHandler` also defines `provide_state` / `retrieve_state` pairs for both
the file system + GLOBALV and the SMSG sender, enabling the REXX interpreter
to issue CMS commands that mutate shared state during EXEC execution.

## The execute() Flow

```
execute(input)
  |
  +-- parse_cms_command(input)
  |     |
  |     +-- OK(cmd)  -->  dispatch(cmd)   // built-in command
  |     |
  |     +-- Err(UnknownCommand)
  |           |
  |           +-- ext_handler.try_execute(input)
  |           |     |
  |           |     +-- Some(rc, msgs)  -->  return result
  |           |     +-- None            -->  fall through
  |           |
  |           +-- try_exec_fallback()   // search <CMD> EXEC *
  |
  +-- Err(other)  -->  RC=24, error message
```

1. **Parse** -- `parse_cms_command` tokenizes the input line, matches the
   first word against the abbreviation table, and returns a `CmsCommand` enum.
2. **Built-in** -- Known commands (LISTFILE, STATE, COPYFILE, ERASE, RENAME,
   GLOBALV, ACCESS, RELEASE, EXEC, SMSG) are dispatched directly.
3. **Extension** -- If parsing yields `UnknownCommand`, the `ExtCommandHandler`
   gets first crack. This is how `cms-spool` and `cms-pipelines` inject their
   commands without modifying cms-core.
4. **EXEC fallback** -- If the extension handler returns `None`, the processor
   searches for `<CMD> EXEC *` on disk and runs it through the `ExecHandler`.

## Abbreviation Matching

Commands follow IBM CMS abbreviation conventions. Each command has a minimum
abbreviation length defined in `CMS_COMMAND_TABLE`:

| Command | Min. Abbrev | Example |
|---|---|---|
| ACCESS | 2 | `AC` |
| COPYFILE | 4 | `COPY` |
| ERASE | 2 | `ER` |
| EXEC | 4 | `EXEC` |
| GLOBALV | 4 | `GLOB` |
| LISTFILE | 4 | `LIST` |
| RELEASE | 3 | `REL` |
| RENAME | 3 | `REN` |
| SMSG | 2 | `SM` |
| STATE | 2 | `ST` |

`lookup_command` tries an exact match first, then checks whether the input is
at least `min_abbrev` characters long and is a prefix of the full command name.
Ambiguous abbreviations (matching more than one command) return no match.
