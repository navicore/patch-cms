# Roadmap

See the detailed phase-by-phase roadmap in the repository root:
[ROADMAP.md](../ROADMAP.md).

## Current State

Phases 1-13 complete. 906+ tests, zero clippy warnings.

All major subsystems are functional:
- XEDIT editor with REXX macros, prefix commands, target system, file ring
- CMS file system (fn ft fm), command processor, GLOBALV
- Spool subsystem (reader/punch/printer)
- Hartmann pipelines (PIPE command with built-in stages)
- IUCV actor framework (Supervisor, SMSG, bidirectional paths)
- Interactive CMS console with REXX/REPL programming

## Known Next Steps

- Phase 13 items still marked in-progress in root ROADMAP.md:
  - `ExtCommandHandler` trait wiring
  - Persistent REXX state across EXECs (`CmsRexxExecHandlerWithSwap`)
  - SMSG from REXX context
  - Nested EXEC support
  - Full spool + pipeline availability from REPL and REXX
- Context-sensitive HELP beyond basic command list
- Additional pipeline stages beyond `literal`, `console`, `locate`, `nlocate`
