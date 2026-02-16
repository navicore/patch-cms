# patch-cms

A Rust reimplementation of the IBM VM/CMS environment — starting with the XEDIT
editor and the REXX macro language, building toward the CMS file system,
Hartmann pipelines, and VM inter-machine messaging.

## Overview

VM/CMS was IBM's interactive mainframe operating system — a single-user virtual
machine with a powerful command environment, a programmable full-screen editor
(XEDIT), and REXX as its scripting language. This project recreates those
semantics in modern Rust as a set of embeddable libraries with a terminal UI.

The REXX interpreter lives in a companion project,
[patch-rexx](https://github.com/navicore/patch-rexx).

See [ROADMAP.md](ROADMAP.md) for the full vision and current progress.

## Workspace Structure

```
patch-cms/
├── crates/
│   ├── xedit-core/          # Editor model — pure logic, no I/O dependencies
│   ├── xedit-tui/           # Terminal UI — 3270-style block-mode rendering
│   ├── cms-core/   (future) # CMS file system (fn ft fm), commands, EXEC processor
│   ├── cms-pipelines/ (future) # Hartmann pipelines
│   └── vm-iucv/    (future) # Inter-machine messaging (actor framework)
```

**xedit-core** is a standalone library with zero I/O dependencies. The editor is
a pure state machine driven by commands, making it embeddable in other
applications. With the `rexx` feature enabled, macros can query and drive the
editor via EXTRACT variables and ADDRESS XEDIT commands.

**xedit-tui** provides the interactive terminal experience: prefix area editing,
command line, PF keys, and screen editing with overtype/insert modes.

## Building and Running

```sh
# Build the workspace
cargo build --workspace

# Build with REXX macro support
cargo build --workspace --features rexx

# Run the TUI editor
cargo run -p xedit-tui -- <filename>

# Run all tests
cargo test --all-features --workspace
```

## Current Status

**XEDIT editor** (Phases 1-3 complete):
- Editor core with full prefix command model (d, dd, i, a, c, cc, m, mm, ", "", >, <, /, f, p)
- Target system: `:n`, `+n`, `-n`, `/string/`, `-/string/`, `*`, compound targets
- Commands: LOCATE, CHANGE, DELETE, INPUT, FILE, SAVE, QUIT, GET, SORT, ALL, and more
- Screen editing: 3270-style block mode with prefix area, data area, and command line
- File ring for multiple open files
- PF key assignments, command history, undo
- REXX macro integration: EXTRACT variables, ADDRESS XEDIT command routing, PROFILE XEDIT, SET MACRO PATH

**Coming next**: CMS file system, command processor, and Hartmann pipelines.

## License

[MIT](LICENSE) — Ed Sweeney, 2026
