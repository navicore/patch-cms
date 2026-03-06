# XEDIT / CMS Reimplementation Roadmap

A Rust implementation of the IBM VM/CMS XEDIT editor and (eventually) the CMS
environment, designed as an embeddable library with a terminal UI.

## Vision

Recreate the power of the VM/CMS programming environment in a modern,
embeddable form:

- **XEDIT**: Full-screen editor with prefix commands, target system, REXX macros
- **CMS**: File system (fn ft fm), command processor, EXEC/REXX scripting
- **VM**: Inter-machine messaging (IUCV) as an actor framework, spool system
- **CMS Pipelines**: Hartmann pipelines for data transformation

The REXX interpreter lives in `../patch-rexx` (v0.9.3, ~15K lines, ANSI-compliant,
embeddable Rust library with full PARSE/INTERPRET/SIGNAL/TRACE support).

## Workspace Architecture

```
patch-cms/
├── crates/
│   ├── xedit-core/                  # Editor model — pure logic, no I/O deps
│   ├── xedit-tui/                   # Terminal UI — 3270-style rendering
│   ├── cms-core/                    # CMS file system, commands, EXEC processor
│   ├── cms-spool/                   # Reader/punch/printer spool subsystem
│   ├── cms-pipelines/               # Hartmann pipelines
│   ├── vm-iucv/                     # Inter-machine messaging (actor framework)
│   └── cms-machine/                 # Interactive CMS machine binary
```

Key design principle: **trait-based seams**. XEDIT depends on a `FileSystem`
trait, not on CMS directly. A `NativeFs` adapter works today; CMS provides the
`fn ft fm` implementation when composed together. This keeps xedit-core
genuinely embeddable standalone.

## Phase 1: Editor Core — DONE

- [x] Buffer model: 1-based line numbering, TOF/EOF virtual positions
- [x] Target system: `:n`, `+n`, `-n`, `/string/`, `-/string/`, `*`
- [x] Compound targets: `/foo/ & /bar/`, `/foo/ | /bar/`
- [x] Command parser with IBM-style abbreviation table (26 commands)
- [x] Navigation: UP, DOWN, TOP, BOTTOM, FORWARD, BACKWARD, LEFT, RIGHT
- [x] Search: LOCATE with targets, `/string/` shorthand
- [x] Editing: CHANGE /old/new/ (with count and target), INPUT, DELETE
- [x] File operations: FILE, SAVE, QUIT, QQUIT, GET
- [x] Settings: SET NUMBER/PREFIX/SCALE/CASE/CURLINE/TRUNC/ZONE/WRAP/STAY/HEX/SHADOW/VERIFY/MSGLINE
- [x] Display: SET RESERVED, SET COLOR (7 areas), REFRESH
- [x] Query: QUERY SIZE/LINE/COL/ALT/TRUNC/LRECL/RECFM
- [x] PF key assignments: SET PF1-24, default IBM assignments
- [x] Prefix commands: d, dd, i, a, c, cc, m, mm, ", "", >, <, /, f, p
- [x] Prefix execution with priority ordering (block ops before singles)
- [x] ALL command (filter visible lines by target)
- [x] SORT command (ascending/descending, column range)
- [x] STACK / QUEUE (data stack for REXX interop)
- [x] CURSOR command (HOME, FILE line col)
- [x] Command history (recall with ?, re-execute with =)
- [x] Multi-level undo

## Phase 2: Screen Editing (3270 Block-Mode Simulation) — DONE

- [x] CursorFocus model: CommandLine vs FileArea
- [x] Tab cycles: CommandLine -> prefix -> data -> CommandLine; Shift-Tab reverses
- [x] Prefix area editing: cursor in cols 1-5, overtype with command text
- [x] Data area editing: overtype mode (default) and insert mode (Insert key)
- [x] Character-level editing: overtype_char, insert_char, delete_char, Backspace
- [x] Enter in file area: batch-process all pending prefixes + command line
- [x] Home/End, PageUp/PageDown, Escape in all focus modes
- [x] ID line: filename, filetype, filemode, Trunc, Size, Line, Col, Alt, [Ins/Ovr]
- [x] 3270-style colors: blue ID line, green data, cyan prefix, yellow current line
- [x] Shadow lines for ALL filter, reserved lines for SET RESERVED

## Phase 3: REXX Macro Integration — DONE

- [x] EXTRACT variables (24 stems): CURLINE, SIZE, LINE, COLUMN, FNAME, FTYPE, FMODE, TRUNC, ALT, TOF, EOF, MODIFIED, LRECL, RECFM, NUMBER, PREFIX, SCALE, CASE, WRAP, HEX, STAY, SHADOW, VERIFY, LASTMSG
- [x] COMMAND interface: macros call XEDIT via `'COMMAND LOCATE /foo/'`
- [x] ADDRESS XEDIT routing with IBM-style RC codes (0/1/2/3/5)
- [x] SET MACRO PATH — search directories for .xedit macros
- [x] MACRO command — load and execute named macros
- [x] PROFILE XEDIT — auto-run on file open
- [x] Macro arguments via `parse arg`

**Limitation:** EXTRACT variables are a static snapshot. Mid-macro changes not
reflected. Use QUERY for fresh state.

## Phase 4: CMS Core — DONE

- [x] FileSpec type: fn ft fm parsing, validation (8-char limits, wildcard matching)
- [x] Minidisk model: directory-backed disks, ReadOnly/ReadWrite access modes
- [x] CmsFileSystem: read, write, state, listfile, erase, copyfile, rename
- [x] Command processor: CMS command line with abbreviation table (9 commands)
- [x] Commands: LISTFILE, STATE, COPYFILE, ERASE, RENAME, ACCESS, RELEASE, EXEC, GLOBALV
- [x] GLOBALV: session-scoped variable storage with groups (SELECT, SET, GET, LIST, DELETE, PURGE)
- [x] PROFILE EXEC: startup macro execution
- [x] FileSystem trait adapter: CmsFs wraps CmsFileSystem for xedit-core

## Phase 5: CMS Mode Composition — DONE

- [x] FileSystem trait seam: xedit-core depends on trait, not CMS directly
- [x] CmsFs adapter: implements xedit-core FileSystem for CMS fn/ft/fm files
- [x] App::with_cms(): TUI constructor with CommandProcessor + CmsFs
- [x] Disk mounting: A-disk read-write (required), B-Z read-only (optional)
- [x] CMS command fallback: unknown XEDIT commands tried as CMS commands
- [x] `--cms <base_path>` CLI flag for CMS mode

## Phase 6: File Ring in TUI + XEDIT Command — DONE

- [x] Ring struct: add_file, cycle_next, prev, remove_current, switch_to_file
- [x] App uses Ring instead of single Editor
- [x] XEDIT command: `X filename` opens file, `X` cycles ring, `X existing` switches
- [x] Ring-aware QUIT/FILE/QQUIT: removes from ring, switches to next, exits on last
- [x] Ring position display in ID line ("Ring 2/3")
- [x] create_cms_fs() factory for additional ring files in CMS mode
- [x] PROFILE XEDIT runs on each new file opened via ring
- [x] reset_for_current_editor() on ring switch (cursor, prefix, command state)

## Phase 7: Editor Polish & Missing Commands — DONE

All commands and display features implemented. HELP is basic (command list);
context-sensitive help deferred to a future enhancement pass.

### Commands
- [x] REPLACE — overwrite current line (used by macros like CENTER.XEDIT)
- [x] PUT — write block of lines to a file
- [x] TRANSFER — copy lines between ring files (non-destructive copy)
- [x] MERGE — interleave lines from another file
- [x] COMPRESS / EXPAND — tab compression/expansion with zone support
- [x] DUPLICAT (command form, not prefix) — duplicate current line n times
- [x] COVERWRITE / CINSERT — column-aware insert/overtype
- [x] RESET — clear pending prefix operations, block markers, and ALL filter

### Display & Settings
- [x] HEX display mode rendering (high/low nibble lines, DarkGray styling)
- [x] SCALE line rendering (IBM XEDIT-style column ruler, Cyan styling)
- [x] WRAP display mode (continuation lines with verify filter support)
- [x] SET ZONE enforcement in COMPRESS/EXPAND
- [x] SET VERIFY column range enforcement in display
- [x] SET TABLINE — tab stop display with verify offset

### Ring enhancements
- [x] XEDIT command with file cycling and switching (`X`, `X filename`)

### HELP facility
- [x] HELP command — basic command list
- [x] Context-sensitive help (HELP SET, HELP LOCATE, etc.)

### Quality
- [x] EXTRACT dynamic refresh (patch-rexx 0.9.4 `set_command_handler_with_env`)

## Phase 8: CMS Spool System — DONE

New crate: `cms-spool/` — 143 tests, zero clippy warnings.

- [x] Virtual reader/punch/printer with `SpoolDevice` enum
- [x] SPOOL command to configure (CLASS, DEST, COPIES, HOLD, CONT/NOCONT)
- [x] RECEIVE/SENDFILE for inter-machine communication
- [x] `SpoolBackend` trait with `InMemoryBackend` and `DirectoryBackend`
- [x] `.meta`/`.data` file pairs for persistent spool entries
- [x] QUERY RDR/PRT/PUN with class filtering
- [x] PURGE with spool-id selection
- [x] `validate_enqueue_fields` with CMS-legal character enforcement
- [x] Transfer-to-reader with rollback on failure
- [x] Orphaned entry cleanup in directory scans

## Phase 9: CMS Pipelines (Hartmann Pipelines) — DONE

New crate: `cms-pipelines/` — 88 tests, zero clippy warnings.

- [x] Pipeline parser: `PIPE stage1 args | stage2 | ...` syntax
- [x] Two-pass executor: initialize → process → finish
- [x] Multi-stream support (primary/secondary outputs with RC codes)
- [x] Built-in stages: `literal`, `console`, `locate`, `nlocate`
- [x] Stage trait with pass-through default for custom stages
- [x] CMS-style RC codes (0/4/24/28/32)

## Phase 10: VM Inter-Machine Messaging (IUCV Actor Framework) — DONE

New crate: `vm-iucv/` — 77 tests, zero clippy warnings.

- [x] Supervisor (Control Program): Tokio-based machine lifecycle management
- [x] MachineHandler trait: `on_ipl`, `on_smsg`, `on_logoff` callbacks
- [x] MachineContext: runtime API (`try_send_smsg`, `sever_path`, `iucv_send`)
- [x] SMSG: single-line message delivery between machines
- [x] IUCV paths: connection lifecycle (pending → established → severed)
- [x] MachineId validation and routing
- [x] Actor model: each machine is an isolated Tokio task

## Phase 11: Documentation — DONE

mdBook reference documentation with CI integration.

- [x] mdBook setup (`book.toml`, `docs/SUMMARY.md`)
- [x] Quick start guide and API reference
- [x] vm-iucv module guide: overview, handlers, IUCV paths, SMSG messaging
- [x] Error codes reference and glossary
- [x] Examples: hello_smsg, echo_server, connection_gating, iucv_chat, multi_machine_pipeline
- [x] GitHub Actions CI for automated mdBook builds

## Phase 12: CMS Machine (Interactive Console) — DONE

New crate: `cms-machine/` — 22 tests (18 unit + 4 integration), zero clippy warnings.

- [x] `CmsMachineHandler`: wraps CommandProcessor, implements MachineHandler
- [x] Interactive console: stdin → commands → handler → stdout
- [x] BATCH_DONE sentinel: reliable sync between console and handler
- [x] $CON wake pattern: console wakes machine via SMSG from $CON pseudo-machine
- [x] Inbound SMSG: messages from other machines stored in GLOBALV LASTING
- [x] ChannelSmsgSender: routes SMSG commands through actor framework
- [x] REXX EXEC support: PROFILE EXEC on startup, ADDRESS CMS in REXX
- [x] CLI: `--userid NAME`, `--disk PATH`, auto-mount A-Z disks
- [x] LOGOFF command with supervisor shutdown

## Phase 13: REXX/REPL Programming Environment — IN PROGRESS

Wire all subsystems together so everything is programmable from REXX/REPL.

- [ ] ExtCommandHandler trait in cms-core (extension point for spool/pipeline)
- [ ] CmsRexxExecHandlerWithSwap: persistent state across REXX EXECs
- [ ] SMSG from REXX: thread SmsgSender into REXX execution context
- [ ] Nested EXEC: REXX calling EXEC that calls EXEC
- [ ] Spool + pipeline commands available from REPL and REXX
- [ ] Full wiring in main.rs and handler.rs

## Design Principles

1. **Embeddable first**: xedit-core has zero I/O dependencies by default.
   The editor model is a pure state machine driven by commands.

2. **Trait seams**: Filesystem, I/O, and display are behind traits.
   Swap implementations for testing, embedding, or CMS integration.

3. **Faithful semantics**: Follow IBM XEDIT behavior where documented.
   Use THE, KEDIT, and IBM manuals as reference. Abbreviation rules,
   command syntax, prefix commands, and target system should feel right
   to someone who used the real thing.

4. **Modern Rust idioms**: No unsafe, strong types, comprehensive tests.
   The codebase should be approachable for Rust developers who never
   touched a mainframe.

5. **Incremental delivery**: Each phase produces something usable.
   Phase 1 = working editor. Phase 3 = programmable editor. Phase 4+ = CMS.

## Current Status

**895 tests passing, zero clippy warnings.**

Phases 1-12 complete. The editor is fully functional with REXX macros, CMS file
system, spool subsystem, Hartmann pipelines, actor-based inter-machine messaging,
and an interactive CMS console. Phase 13 wires the remaining subsystems together
so everything is programmable from REXX/REPL without dropping into Rust.
