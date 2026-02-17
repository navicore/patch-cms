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
patch-xedit/
├── crates/
│   ├── xedit-core/                  # Editor model — pure logic, no I/O deps
│   ├── xedit-tui/                   # Terminal UI — 3270-style rendering
│   ├── cms-core/                    # CMS file system, commands, EXEC processor
│   ├── cms-spool/         (future)  # Reader/punch/printer spool subsystem
│   ├── cms-pipelines/     (future)  # Hartmann pipelines
│   └── vm-iucv/           (future)  # Inter-machine messaging (actor framework)
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

## Phase 7: Editor Polish & Missing Commands

The editor works but lacks several commands present in real XEDIT. This phase
fills gaps that users of the real editor would notice.

### Commands
- [ ] REPLACE — overwrite current line (used by macros like CENTER.XEDIT)
- [ ] PUT — write block of lines to a file
- [ ] TRANSFER — copy lines between ring files
- [ ] MERGE — interleave lines from another file
- [ ] COMPRESS / EXPAND — tab compression
- [ ] DUPLICAT (command form, not prefix) — duplicate current line n times
- [ ] COVERWRITE / CINSERT — column-aware insert/overtype
- [ ] RESET — clear pending prefix operations and block markers

### Display & Settings
- [ ] HEX display mode rendering (currently parsed but not rendered)
- [ ] SCALE line rendering (currently parsed but not rendered)
- [ ] WRAP display mode (currently parsed but not rendered)
- [ ] SET ZONE enforcement in LOCATE and CHANGE
- [ ] SET VERIFY column range enforcement in display
- [ ] SET TABLINE — tab stop display

### Ring enhancements
- [ ] XEDIT command with CMS-style file ID (`X PROFILE EXEC A` parsing)

### HELP facility
- [ ] HELP command — display help text for commands
- [ ] Context-sensitive help (HELP SET, HELP LOCATE, etc.)

### Quality
- [ ] EXTRACT dynamic refresh (requires patch-rexx callback enhancement)

## Phase 8: CMS Spool System

New crate: `cms-spool/`

- Virtual reader/punch/printer
- SPOOL command to configure
- RECEIVE/SENDFILE for inter-machine communication
- Map to real I/O: files, network sockets, message queues
- Reader -> input stream (stdin, files, network)
- Printer -> output stream (stdout, files, log)
- Punch -> binary output stream

## Phase 9: CMS Pipelines (Hartmann Pipelines)

New crate: `cms-pipelines/`

The most underappreciated tool in computing history.

- `PIPE` command to define pipeline stages
- Built-in stages: `< file`, `> file`, `locate`, `nlocate`, `change`,
  `count`, `sort`, `unique`, `specs`, `pad`, `strip`, `xlate`, `console`,
  `stack`, `stem`, `var`, `literal`, `chop`, `join`, `split`, `fanout`,
  `faninany`, `gate`, etc.
- Multi-stream pipelines (primary + secondary outputs)
- Pipeline stages as Rust iterators/async streams
- User-written stages in REXX

### Example
```
pipe < data.txt | locate /ERROR/ | change /ERROR/WARNING/ | > fixed.txt
pipe < log.txt | locate /ERROR/ | count lines | console
```

## Phase 10: VM Inter-Machine Messaging (Actor Framework)

New crate: `vm-iucv/`

- Typed message passing between CMS "machines"
- Each CMS machine = a Tokio task with its own Environment
- IUCV CONNECT / SEND / RECEIVE semantics
- SMSG (single-line message) for simple communication
- Map to: channels, network sockets, gRPC, NATS, etc.
- This is the actor model — each machine is an isolated actor
- Supervisor patterns for machine lifecycle

### Connection to Go CSP patterns
- VM/CMS IUCV ~ Go channels between goroutines
- Each CMS machine ~ a goroutine with isolated state
- SMSG ~ simple channel send
- IUCV paths ~ typed bidirectional channels

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

**281 tests passing, zero clippy warnings.**

Phases 1-6 complete. The editor is fully functional with screen editing,
REXX macros, CMS file system integration, and multi-file ring support.
Phase 7 is the next natural step — filling in missing XEDIT commands and
rendering modes that real users would expect.
