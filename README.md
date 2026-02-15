# patch-xedit

A Rust reimplementation of the IBM VM/CMS XEDIT full-screen editor.

## Overview

XEDIT was the primary editor on IBM VM/CMS mainframes — a powerful,
prefix-command-driven editor with a target system, macro language (REXX), and
block-mode screen editing. This project recreates those semantics in modern Rust
as an embeddable library with a terminal UI.

See [ROADMAP.md](ROADMAP.md) for the full vision, including planned CMS file
system, Hartmann pipelines, and VM inter-machine messaging.

## Workspace Structure

```
patch-xedit/
├── crates/
│   ├── xedit-core/   # Editor model — pure logic, no I/O dependencies
│   └── xedit-tui/    # Terminal UI — 3270-style block-mode rendering
```

**xedit-core** is a standalone library with zero I/O dependencies. The editor is
a pure state machine driven by commands, making it embeddable in other
applications.

**xedit-tui** provides the interactive terminal experience: prefix area editing,
command line, PF keys, and screen editing with overtype/insert modes.

## Building and Running

```sh
# Build the workspace
cargo build --workspace

# Run the TUI editor
cargo run -p xedit-tui -- <filename>

# Run all tests
cargo test --workspace
```

## Current Status

- Editor core with full prefix command model (d, dd, i, a, c, cc, m, mm, ", "", >, <, /, f, p)
- Target system: `:n`, `+n`, `-n`, `/string/`, `-/string/`, `*`, compound targets
- Commands: LOCATE, CHANGE, DELETE, INPUT, FILE, SAVE, QUIT, GET, SORT, ALL, and more
- Screen editing: 3270-style block mode with prefix area, data area, and command line
- File ring for multiple open files
- PF key assignments, command history, undo

## License

[MIT](LICENSE) — Ed Sweeney, 2026
