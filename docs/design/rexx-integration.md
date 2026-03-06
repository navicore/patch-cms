# REXX Integration

## The Problem

REXX is a 1979 scripting language designed for interactive mainframe
use. It assumes it owns the process, can call any system command by
name, and shares mutable state freely with its host. Rust's ownership
model is the opposite of all of that.

## Feature-Gated Dependency

REXX support is behind the `rexx` feature flag in both xedit-core and
cms-machine. Without it, the crates compile with no scripting support
and `NoExecHandler` stubs return RC 28 ("not found") for any EXEC
invocation. This keeps the core editor embeddable without pulling in
the full REXX interpreter.

```toml
# In xedit-core/Cargo.toml
[features]
rexx = ["dep:patch-rexx"]
```

The REXX interpreter itself lives in `patch-rexx`, a separate
repository (~15K lines, ANSI-compliant).

## Two Integration Points

### ADDRESS XEDIT (Editor Macros)

When a REXX macro runs inside the editor, it needs to:

1. **Read editor state** via EXTRACT variables (CURLINE, SIZE, FNAME, etc.)
2. **Issue editor commands** via bare string expressions like
   `'COMMAND LOCATE /foo/'`

The bridge lives in `xedit-core/src/macro_engine.rs`. Before
execution, we populate REXX compound variables with a snapshot of
editor state. A custom `set_command_handler` closure intercepts bare
string expressions and routes them to the editor's command processor.

**State snapshot, not live binding.** EXTRACT variables are populated
once at macro start. If the macro changes the buffer (e.g., via
CHANGE), the EXTRACT values do not update mid-execution. This is a
known limitation documented in the ROADMAP. Use QUERY for fresh state.

### ADDRESS CMS (System Commands)

When REXX runs in the CMS machine context, it needs to call CMS
commands (LISTFILE, PIPE, spool operations). This is wired through
the `ExecHandler` and `ExtCommandHandler` traits:

```
patch-rexx (interpreter)
    |
    v
CmsRexxExecHandler (cms-machine) -- implements ExecHandler trait
    |
    v
CommandProcessor (cms-core) -- dispatches commands
    |
    v
ExtCommandHandler (cms-core trait) -- extension point
    |
    v
CmsExtCommandHandler (cms-machine) -- routes PIPE, spool commands
```

Each layer depends only on the trait above it, never on a concrete
type from another crate.

## The State Swap Pattern

REXX macros in CMS mode need access to the CMS filesystem and GLOBALV
variables. But `CommandProcessor` owns that state, and the REXX
interpreter needs it too during execution.

`CmsRexxExecHandlerWithSwap` solves this with explicit ownership
transfer:

1. **Before EXEC:** `provide_state(fs, globalv)` -- CommandProcessor
   gives its filesystem and variable state to the handler
2. **During EXEC:** REXX runs with full access to CMS files and
   variables
3. **After EXEC:** `retrieve_state()` -- handler returns the
   (potentially modified) state back to CommandProcessor

This avoids `Arc<Mutex<>>` and keeps the borrow checker happy. The
state is never shared -- it moves from owner to owner.

## SMSG from REXX

The `SmsgSender` trait lets REXX programs send inter-machine messages:

```rust
pub trait SmsgSender {
    fn send_smsg(&self, target: &str, text: &str) -> i32;
}
```

`ChannelSmsgSender` (in cms-machine) implements this by enqueuing
messages to the vm-iucv router channel. This bridges the synchronous
REXX execution context with the asynchronous actor framework without
blocking the Tokio runtime.

## Design Constraints

- **No persistent REXX state between EXECs.** Each EXEC invocation
  gets a fresh interpreter instance. Persistent state goes through
  GLOBALV.

- **No concurrent REXX execution.** A CMS machine processes one
  command at a time, including REXX EXECs. This matches real CMS
  behavior and avoids the need to make the interpreter thread-safe.

- **REXX is always optional.** Any crate that works with REXX also
  works without it. The trait seam pattern makes this possible without
  `#[cfg]` spaghetti -- the no-op implementation handles the
  feature-off case.
