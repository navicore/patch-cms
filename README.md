# patch-cms

**[Documentation](https://www.navicore.tech/patch-cms/)**

A Rust reimplementation of the IBM VM/CMS environment — XEDIT editor, CMS file
system, REXX scripting, spool subsystem, Hartmann pipelines, and VM
inter-machine messaging.

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
patch-xedit/
├── crates/
│   ├── xedit-core/          # Editor model — pure logic, no I/O dependencies
│   ├── xedit-tui/           # Terminal UI — 3270-style block-mode rendering
│   ├── cms-core/            # CMS file system (fn ft fm), commands, EXEC processor
│   ├── cms-spool/           # Reader/punch/printer spool subsystem
│   ├── cms-pipelines/       # Hartmann pipelines
│   ├── vm-iucv/             # Inter-machine messaging (actor framework)
│   └── cms-machine/         # Interactive CMS machine binary
```

**xedit-core** is a standalone library with zero I/O dependencies. The editor is
a pure state machine driven by commands, making it embeddable in other
applications. With the `rexx` feature enabled, macros can query and drive the
editor via EXTRACT variables and ADDRESS XEDIT commands.

**xedit-tui** provides the interactive terminal experience: prefix area editing,
command line, PF keys, and screen editing with overtype/insert modes.

**cms-core** implements the CMS file system (`fn ft fm` naming), command
processor with IBM-style abbreviation matching, GLOBALV variable storage, and
EXEC resolution. Trait seams (`ExecHandler`, `SmsgSender`, `ExtCommandHandler`)
decouple it from the REXX interpreter, actor framework, and extension commands.

**cms-spool** provides virtual reader/punch/printer queues with SPOOL, QUERY,
PURGE, SENDFILE, and RECEIVE commands.

**cms-pipelines** implements Hartmann pipelines — the `PIPE` command with
built-in stages (`literal`, `console`, `locate`, `nlocate`) and a two-pass
executor.

**vm-iucv** is a Tokio-based actor framework modeled after VM/CMS inter-machine
communication: Supervisor lifecycle management, SMSG messaging, and IUCV
bidirectional data paths.

**cms-machine** wires everything together into an interactive CMS console with
REXX scripting, spool commands, and pipelines — all programmable from the REPL.

## Building and Running

```sh
# Build the workspace
cargo build --workspace

# Run the TUI editor
cargo run -p xedit-tui -- <filename>

# Run the interactive CMS machine
cargo run -p cms-machine -- --userid ALICE --disk /tmp/cms

# Run all tests
cargo test --workspace
```

## Current Status

**906 tests passing, zero clippy warnings.**

Phases 1-13 complete. The editor is fully functional with REXX macros, CMS file
system, spool subsystem, Hartmann pipelines, actor-based inter-machine messaging,
and an interactive CMS console. Everything is programmable from REXX/REPL.

## License

[MIT](LICENSE) — Ed Sweeney, 2026
