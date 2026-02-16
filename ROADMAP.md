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

The REXX interpreter lives in `../patch-rexx` (v0.9.1, ~15K lines, ANSI-compliant,
embeddable Rust library with full PARSE/INTERPRET/SIGNAL/TRACE support).

## Workspace Architecture

```
patch-xedit/                        (may rename to patch-cms)
├── crates/
│   ├── xedit-core/                  # Editor model — pure logic, no I/O deps
│   ├── xedit-tui/                   # Terminal UI — 3270-style rendering
│   ├── cms-core/          (future)  # CMS file system, commands, EXEC processor
│   ├── cms-spool/         (future)  # Reader/punch/printer spool subsystem
│   ├── cms-pipelines/     (future)  # Hartmann pipelines
│   └── vm-iucv/           (future)  # Inter-machine messaging (actor framework)
```

Key design principle: **trait-based seams**. XEDIT depends on a `FileSystem`
trait, not on CMS directly. A `NativeFs` adapter works today; CMS provides the
`fn ft fm` implementation when composed together. This keeps xedit-core
genuinely embeddable standalone.

## Phase 1: Editor Core (CURRENT — foundation complete)

### Done
- [x] Buffer model: 1-based line numbering, TOF/EOF virtual positions
- [x] Target system: `:n`, `+n`, `-n`, `/string/`, `-/string/`, `*`
- [x] Command parser with IBM-style abbreviation table (L=LOCATE, C=CHANGE, etc.)
- [x] Navigation: UP, DOWN, TOP, BOTTOM, FORWARD, BACKWARD, LEFT, RIGHT
- [x] Search: LOCATE with targets, `/string/` shorthand
- [x] Editing: CHANGE /old/new/, INPUT (with input mode), DELETE with targets
- [x] File operations: FILE, SAVE, QUIT, QQUIT, GET
- [x] Settings: SET NUMBER/PREFIX/SCALE/CASE/CURLINE/TRUNC/WRAP/STAY/HEX
- [x] Query: QUERY SIZE/LINE/COL/ALT/TRUNC/LRECL/RECFM
- [x] Prefix command model: d, dd, i, a, c, cc, m, mm, ", "", >, <, /, f, p
- [x] Prefix execution: delete, insert, duplicate, shift, block ops, copy/move
- [x] File ring: multiple open files, cycle through
- [x] 133 unit tests passing

### TODO — Phase 1 remaining
- [x] **Screen editing model** (completed in Phase 2)
- [x] Compound targets: `/foo/ & /bar/`, `/foo/ | /bar/`
- [x] ALL command (show only lines matching target)
- [x] SORT command
- [x] STACK / QUEUE (data stack for REXX interop)
- [x] CURSOR command
- [x] SET SHADOW, SET RESERVED, SET COLOR
- [x] EXTRACT command (via REXX macro variable population)
- [x] COMMAND vs MACRO resolution
- [x] PF key assignments (SET PFn command)
- [x] Command history (recall previous commands)
- [x] Undo (multi-level)

## Phase 2: Screen Editing (3270 Block-Mode Simulation) — DONE

### Done
- [x] CursorFocus model: CommandLine vs FileArea
- [x] Tab cycles: CommandLine → FileArea (prefix) → FileArea (data) → CommandLine
- [x] Shift-Tab reverses the cycle
- [x] Arrow keys move freely within file area (up/down between lines, left/right between columns)
- [x] Arrow left/right skips separator column between prefix and data
- [x] Prefix area editing: cursor in cols 1-5, overtype line number with command text
- [x] Per-line prefix input buffers (HashMap), visual feedback (bold white prefix text)
- [x] Data area editing: overtype mode (default) and insert mode (Insert key toggles)
- [x] Character-level editor methods: overtype_char, insert_char, delete_char
- [x] Backspace works in both prefix and data areas
- [x] Delete key removes character at cursor in data area
- [x] Home/End: move to start/end within current area
- [x] Enter in file area: batch-process all pending prefix commands + command line
- [x] Escape in file area: clear pending prefixes, return to command line
- [x] Cursor positioned correctly in all areas (command line, prefix, data)
- [x] ID line shows [Ins]/[Ovr] mode indicator
- [x] Message line shows contextual help when no message pending
- [x] Arrow up/down in command line still scrolls (moves current line)
- [x] PageUp/PageDown work in both focus modes

### TODO — refinements
- [x] Priority-ordered prefix processing (block ops before singles)

### Implementation approach
- Add `CursorFocus` enum: `CommandLine | Prefix(line_num) | Data(line_num, col)`
- Per-line prefix input buffer in the app state (not in editor core)
- Arrow keys: move cursor within and between areas
- Tab: cycle CommandLine → Prefix → Data → CommandLine
- Enter: collect all prefix inputs, execute them, then execute command line
- Home/End: move within current area

## Phase 3: REXX Macro Integration — DONE

Wire `patch-rexx` as a dependency of `xedit-core`.

### Done
- [x] EXTRACT variables: CURLINE, SIZE, LINE, COLUMN, FNAME, FTYPE, FMODE, TRUNC, ALT, TOF, EOF, MODIFIED
- [x] EXTRACT variables (added): LRECL, RECFM, NUMBER, PREFIX, SCALE, CASE, WRAP, HEX, STAY, SHADOW, VERIFY, LASTMSG
- [x] COMMAND interface: REXX macros call XEDIT commands via `'COMMAND LOCATE /foo/'`
- [x] ADDRESS XEDIT command routing with IBM-style RC codes (0=success, 1=error, 2=target not found, 3=bad command, 5=file I/O)
- [x] SET MACRO PATH command — configure search directories for .xedit macros
- [x] MACRO command — load and execute named macros
- [x] PROFILE XEDIT — auto-run macro on file open
- [x] Macro arguments via `parse arg`

### Limitation
- EXTRACT variables are a static snapshot taken before macro execution. Mid-macro
  changes (cursor moves, edits) are not reflected. Macros needing fresh state
  should use `QUERY` and parse the message. Dynamic refresh requires a
  `patch-rexx` enhancement (post-command callback or extended handler return type).

### Example macro (what we're targeting)
```rexx
/* CENTER.XEDIT — center text on current line */
'EXTRACT /CURLINE/TRUNC/'
text = strip(curline.3)
pad = (trunc.1 - length(text)) % 2
'COMMAND REPLACE' copies(' ', pad) || text
```

## Phase 4: CMS Core

### File system model
- **fn ft fm** addressing (FILENAME FILETYPE FILEMODE)
- Filemode letters A-Z with access modes (A1 = read/write, etc.)
- LISTFILE, STATE, COPYFILE, ERASE, RENAME commands
- Minidisk concept (directories mapped to filemode letters)
- Could map to real directories: A → $HOME/cms/a/, etc.

### Command processor
- CMS command line with EXEC/REXX resolution
- EXEC: legacy command procedure language
- Search order: EXEC → REXX → builtin → external
- PROFILE EXEC — auto-run on "IPL" (startup)
- GLOBALV — global variable storage across commands

### HELP facility
- HELP command with paneled help text
- Could use markdown files as help source

## Phase 5: CMS Spool System

- Virtual reader/punch/printer
- SPOOL command to configure
- RECEIVE/SENDFILE for inter-machine communication
- Map to real I/O: files, network sockets, message queues
- Reader → input stream (stdin, files, network)
- Printer → output stream (stdout, files, log)
- Punch → binary output stream

## Phase 6: CMS Pipelines (Hartmann Pipelines)

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

## Phase 7: VM Inter-Machine Messaging (Actor Framework)

- `vm-iucv` crate: typed message passing between CMS "machines"
- Each CMS machine = a Tokio task with its own Environment
- IUCV CONNECT / SEND / RECEIVE semantics
- SMSG (single-line message) for simple communication
- Map to: channels, network sockets, gRPC, NATS, etc.
- This is the actor model — each machine is an isolated actor
- Supervisor patterns for machine lifecycle

### Connection to Go CSP patterns
- VM/CMS IUCV ≈ Go channels between goroutines
- Each CMS machine ≈ a goroutine with isolated state
- SMSG ≈ simple channel send
- IUCV paths ≈ typed bidirectional channels

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
