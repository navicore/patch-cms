# Architecture

## Context & Scope

patch-cms is a Rust reimplementation of the IBM VM/CMS environment — the
interactive mainframe OS that combined a single-user virtual machine, the XEDIT
editor, REXX scripting, and inter-machine messaging.

**Inside the boundary:** editor model, CMS file system, spool subsystem,
Hartmann pipelines, IUCV actor framework, and an interactive CMS console — all
as embeddable Rust libraries plus a terminal binary.

**Outside the boundary:**
- **patch-rexx** — companion crate providing the REXX interpreter (optional
  dependency via `rexx` feature flag)
- **Terminal** — crossterm/ratatui for TUI rendering (xedit-tui only)
- **Host filesystem** — minidisks are directory-backed; spool uses `.meta`/`.data`
  file pairs
- **Tokio runtime** — async executor for the actor framework (vm-iucv,
  cms-machine)

## Solution Strategy

- **Rust** — zero `unsafe`, strong types, exhaustive matching. Embeddable
  libraries with no forced runtime dependencies in core crates.
- **Trait seams** — every major subsystem boundary is a trait with a default
  no-op implementation. Crates depend on traits, never on sibling internals.
  This keeps the dependency graph shallow and each crate independently testable.
- **Faithful VM/CMS semantics** — abbreviation rules, RC codes, prefix area
  behavior, and REXX execution model follow IBM documentation.
- **Actor model (Tokio)** — vm-iucv models inter-machine communication as
  isolated Tokio tasks managed by a Supervisor, with SMSG and IUCV path
  primitives.

## Building Blocks

```
cms-machine          # Binary — wires all crates into interactive CMS console
  +-- cms-core       # CMS file system (fn ft fm), command processor, GLOBALV
  +-- cms-spool      # Reader/punch/printer spool queues (no external deps)
  +-- cms-pipelines  # Hartmann PIPE command with built-in stages (no external deps)
  +-- vm-iucv        # Tokio actor framework: Supervisor, SMSG, IUCV paths
  +-- xedit-core     # Editor state machine — pure logic, zero I/O deps
      +-- (patch-rexx)  # Optional: REXX macro engine

xedit-tui            # Standalone TUI binary — 3270-style block-mode editor
  +-- xedit-core
  +-- cms-core       # Optional CMS mode
```

**Core domain entities:**
- `FileSpec` (fn ft fm) — CMS file naming with 8-char limits and wildcard matching
- `Minidisk` — directory-backed disk with ReadOnly/ReadWrite access
- `Editor` / `Buffer` — 1-based line model with TOF/EOF, target system, prefix commands
- `Ring` — multi-file editor ring with cycling
- `SpoolFile` / `SpoolDevice` — virtual reader/punch/printer queues
- `Supervisor` / `MachineId` — actor lifecycle and message routing
- `CommandProcessor` — CMS command dispatch with IBM-style abbreviation matching

**Key trait seams (cms-core):**
- `ExecHandler` — REXX interpreter integration (execute, state swap)
- `SmsgSender` — SMSG message delivery between machines
- `ExtCommandHandler` — extension point for spool/pipeline commands (Option-based fallthrough)
- `FileSystem` (xedit-core) — abstracts file I/O; `CmsFs` adapter provides fn/ft/fm semantics

## Crosscutting Concepts

**Error handling:** Custom error enums per crate with manual `Display`/`Error`
impls (no thiserror/anyhow). Domain errors include IBM-style message prefixes
(DMSSPL*, DMSIUC*) and `.rc()` methods returning CMS return codes.

**Return codes:** Commands return integer RC codes following IBM conventions
(0 = success, 1-5 for XEDIT, 2/4/8/.../100 for CMS/CP). These flow through
REXX and the command processor.

**Concurrency:** Synchronous within each crate except vm-iucv and cms-machine,
which use Tokio. The console uses a `$CON` pseudo-machine SMSG pattern to wake
the handler task. BATCH_DONE sentinel synchronizes console I/O.

**Testing:** Inline `#[cfg(test)]` modules for unit tests; `tests/` directory
for async integration tests in cms-machine. 906+ tests, zero clippy warnings.

**Serialization:** Spool persistence uses `.meta`/`.data` file pairs on disk.
No serde dependency — formats are simple text.
