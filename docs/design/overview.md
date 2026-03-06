# Design Principles

This section documents the architectural decisions behind patch-cms --
the *why*, not the *what*. The ROADMAP tracks features; these pages
explain the reasoning future maintainers should understand before
changing things.

## Core Tenets

1. **Embeddable first.** xedit-core has zero I/O dependencies. The
   editor is a pure state machine driven by commands so it can be
   embedded in any Rust application without pulling in a terminal
   library, an async runtime, or a filesystem.

2. **Trait seams, not concrete coupling.** Every major subsystem
   boundary is a trait with a default no-op implementation. Crates
   depend on traits, never on sibling crate internals. This keeps
   the dependency graph shallow and lets each crate be tested and
   used in isolation.

3. **Faithful VM/CMS semantics.** Abbreviation rules, command
   syntax, RC codes, prefix area behavior, and the REXX execution
   model follow IBM documentation. Where the real system's behavior
   is well-documented, we match it; where it isn't, we pick the
   simplest defensible interpretation.

4. **Modern Rust idioms.** Zero `unsafe` blocks across the entire
   workspace. Strong types, exhaustive pattern matching, and
   comprehensive tests. The codebase should be approachable for
   Rust developers who have never touched a mainframe.

5. **Incremental delivery.** Each phase produces something usable on
   its own. The editor works without CMS; CMS works without the
   spool; the actor framework works without either.

## Design Documents

- [Why Rust](why-rust.md) -- language choice and what it gives us
- [REXX Integration](rexx-integration.md) -- bridging a 1979 scripting
  language with Rust ownership
- [Async Architecture](async-architecture.md) -- the actor model,
  sync/async boundary, and Tokio usage
- [Trait Seams](trait-seams.md) -- the decoupling strategy and crate
  dependency graph
