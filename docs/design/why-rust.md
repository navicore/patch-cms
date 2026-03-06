# Why Rust

## The Choice

VM/CMS is a systems programming environment. Recreating it requires
precise control over memory layout, string handling, and concurrency --
the same concerns that made the original system a C and assembler
project. Rust gives us that control without the footguns.

## What Rust Gives Us

### Zero-Cost Abstractions for Editor Internals

xedit-core is a pure state machine. Every command is a function from
`(EditorState, Command) -> (EditorState, Result)`. Rust's enum-based
command representation and exhaustive `match` mean we can add a new
command and the compiler tells us every place that needs updating.
There is no dynamic dispatch in the hot path.

### Ownership Makes Embeddability Real

The `Editor` struct owns its buffer, settings, and undo stack. There
are no global singletons, no thread-local state, and no hidden
allocations. You can create ten editors in one process, each with its
own filesystem adapter, and they will not interfere. This would be
painful to guarantee in C or C++ and impossible to enforce in a
garbage-collected language.

### Fearless Concurrency in the Actor Framework

vm-iucv runs each machine as an isolated Tokio task. Messages flow
through typed `mpsc` channels. The type system enforces that a
`MachineHandler` callback cannot access another machine's state --
there is no lock to forget, because there is no shared mutable state
to lock. The only synchronization is in the `Supervisor`, which holds
an `Arc<RwLock<HashMap>>` of machine entries.

### Feature Flags for Optional Coupling

Cargo features let us gate expensive dependencies. The `rexx` feature
pulls in `patch-rexx` (~15K lines); without it, xedit-core compiles
in under a second and has no scripting support. The `cms` feature in
xedit-tui pulls in the CMS file system. This keeps compile times
reasonable and lets downstream users take only what they need.

### Safe FFI Boundary (Future)

When we eventually expose a C API for embedding, Rust's `#[no_mangle]`
and `extern "C"` give us a clean FFI surface. The zero-unsafe policy
means the only unsafe code will be at that boundary, making it easy to
audit.

## What We Avoid

- **No `unsafe` anywhere.** The entire workspace compiles without a
  single `unsafe` block. We use `std::mem::swap` instead of pointer
  tricks, `Rc<RefCell<>>` instead of raw pointers for shared mutable
  state in macro execution, and Tokio's typed channels instead of
  lock-free queues.

- **No runtime reflection.** Command dispatch uses Rust enums, not
  string-keyed maps. Abbreviation lookup uses a static table built at
  compile time.

- **No global state.** Every subsystem is instantiated explicitly.
  `CommandProcessor`, `SpoolManager`, `Supervisor` -- all are owned
  values passed by reference. This makes testing trivial: each test
  constructs its own world.
