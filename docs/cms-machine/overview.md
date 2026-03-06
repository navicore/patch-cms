# Overview

The `cms-machine` crate wires all subsystems together into an interactive CMS
console. It is the top-level binary that provides a REPL where users can issue
CMS commands, run REXX EXECs, use spool commands, and execute pipelines.

## CmsMachineHandler

`CmsMachineHandler` implements the `MachineHandler` trait from `vm-iucv`,
bridging the actor framework with the CMS `CommandProcessor`. It handles:

- **on_ipl** -- Prints the startup banner, runs `PROFILE EXEC` if present,
  and begins the command loop.
- **on_smsg** -- Dispatches incoming messages. Messages from `$CON` trigger
  command execution; messages from other machines are displayed and stored
  in GLOBALV (`LMSGSRC`, `LMSGTXT`).
- **on_logoff** -- Cleans up resources.

## Console Architecture

The console uses a split-thread design:

```
stdin (blocking read)
  |
  v
cmd_tx channel ──> CmsMachineHandler
  |                      |
  +── SMSG from $CON ──> on_smsg()
                           |
                           v
                    CommandProcessor::execute()
                           |
                           v
                    output_tx channel ──> stdout
```

1. A dedicated thread reads lines from stdin and sends them via `cmd_tx`.
2. After sending a command, the thread sends an SMSG from the `$CON`
   pseudo-machine to wake the handler.
3. The handler receives the SMSG, pulls the command from `cmd_rx`, executes
   it, and sends output lines via `output_tx`.
4. The console thread reads output lines until it sees the `BATCH_DONE`
   sentinel, then prompts for the next command.

### BATCH_DONE Sentinel

The `BATCH_DONE` constant is a special marker string sent after every command
completes. The console thread waits for this sentinel before displaying the
next prompt, ensuring output from one command is fully flushed before the next
begins.

## Subsystem Integration

All subsystems are available from the REPL:

| Subsystem | How it connects |
|-----------|----------------|
| **CMS commands** | Built into `CommandProcessor` (GLOBALV, LISTFILE, etc.) |
| **REXX EXECs** | `CmsRexxExecHandlerWithSwap` -- persistent state across EXECs |
| **Spool** | `CmsExtCommandHandler` routes SP/QUERY/PURGE/SENDFILE/RECEIVE |
| **Pipelines** | `CmsExtCommandHandler` routes PIPE commands to `run_pipe()` |
| **SMSG** | `ChannelSmsgSender` routes through the actor framework |

The `CmsRexxExecHandlerWithSwap` handler swaps the file system, GLOBALV store,
and SMSG sender into the REXX execution context, so REXX EXECs can issue CMS
commands that modify persistent state.

## CLI Usage

```sh
# Basic usage
cargo run -p cms-machine -- --userid ALICE --disk /tmp/cms

# Multiple disks (mounted as A, B, C, ...)
cargo run -p cms-machine -- --userid BOB --disk /tmp/d1 --disk /tmp/d2
```

The `--userid` flag sets the machine's identity for SMSG routing. Each
`--disk` path is mounted as a minidisk at successive filemode letters
(A, B, C, ...) in read-write mode.
