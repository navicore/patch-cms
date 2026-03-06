# Trait Seams

## The Strategy

Every major boundary between crates is a Rust trait with a default
no-op implementation. Crates depend on traits defined in their own
crate or in a leaf crate, never on sibling crate internals. This
keeps the dependency graph shallow and allows each crate to be
compiled, tested, and used independently.

## The Traits

### FileSystem (xedit-core)

```rust
pub trait FileSystem {
    fn read_file(&self, file_id: &str) -> Result<String>;
    fn write_file(&self, file_id: &str, content: &str) -> Result<()>;
    fn parse_file_id(&self, file_id: &str) -> Option<FileIdentity>;
}
```

Defined in xedit-core. The editor calls this trait -- it never touches
`std::fs` directly.

**Implementations:**
- `NativeFs` (xedit-core) -- delegates to `std::fs`, used in
  standalone XEDIT mode
- `CmsFs` (cms-core) -- maps `fn ft fm` file identifiers to the CMS
  minidisk filesystem

This is the core embeddability seam. A game engine, a web IDE, or a
test harness can implement `FileSystem` and get a working XEDIT editor
without any filesystem at all.

### MachineHandler (vm-iucv)

```rust
pub trait MachineHandler: Send + 'static {
    fn on_ipl(&mut self, ctx: &MachineContext) {}
    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage);
    fn on_logoff(&mut self, ctx: &MachineContext) {}
    // ... IUCV connection callbacks with defaults ...
}
```

Defined in vm-iucv. The Supervisor calls these callbacks -- it knows
nothing about CMS, editors, or files.

**Implementations:**
- `CmsMachineHandler` (cms-machine) -- wraps a CommandProcessor and
  bridges console I/O
- `CollectorHandler` (vm-iucv, test utility) -- collects messages for
  assertions
- Example handlers in vm-iucv/examples/ -- echo servers, pipeline
  stages, connection gating

### Stage (cms-pipelines)

```rust
pub trait Stage {
    fn initialize(&mut self, _output: &mut dyn StageSink) {}
    fn process(&mut self, record: &str, output: &mut dyn StageSink);
    fn finish(&mut self, _output: &mut dyn StageSink) {}
    // ...
}
```

Defined in cms-pipelines. The pipeline executor drives stages through
`initialize → process* → finish` without knowing what they do.

**Built-in implementations:** `literal`, `console`, `locate`, `nlocate`

### SpoolBackend (cms-spool)

```rust
pub trait SpoolBackend {
    fn enqueue(&mut self, ...) -> Result<SpoolId>;
    fn dequeue(&mut self, ...) -> Result<Option<SpoolEntry>>;
    fn list_queue(&self, ...) -> Vec<SpoolEntry>;
    fn purge(&mut self, ...) -> Result<()>;
    // ...
}
```

Defined in cms-spool. The SpoolManager delegates storage to a backend.

**Implementations:**
- `InMemoryBackend` (cms-spool) -- for testing and transient use
- `DirectoryBackend` (cms-spool) -- `.meta`/`.data` file pairs on disk

### ExecHandler, SmsgSender, ExtCommandHandler (cms-core)

Three small traits that let cms-core's `CommandProcessor` call out to
REXX, messaging, and extension commands without depending on their
implementations:

```rust
pub trait ExecHandler {
    fn execute(&mut self, name: &str, args: &str, ...) -> i32;
}

pub trait SmsgSender {
    fn send_smsg(&self, target: &str, text: &str) -> i32;
}

pub trait ExtCommandHandler {
    fn handle(&mut self, command: &str, args: &str, ...) -> Option<i32>;
}
```

Each has a `No*` default (e.g., `NoExecHandler`) that returns a
failure RC. cms-machine provides the real implementations.

## Crate Dependency Graph

```
                    xedit-core
                   /          \
              xedit-tui     cms-core
                   \          /
                    \        /
     cms-spool       \      /      cms-pipelines
          \           \    /           /
           \       cms-machine        /
            \_____|____|_____________/
                  |
               vm-iucv
```

**Leaf crates (no workspace dependencies):**
- `vm-iucv` -- depends only on Tokio
- `cms-spool` -- depends on nothing (only `tempfile` for tests)
- `cms-pipelines` -- depends on nothing

**Middle crates:**
- `xedit-core` -- optionally depends on `patch-rexx`
- `cms-core` -- depends on xedit-core (for FileSystem trait)

**Composition crate:**
- `cms-machine` -- pulls everything together, provides `main()`

This hierarchy means you can use vm-iucv as a standalone actor
framework, cms-pipelines as a standalone data pipeline library, or
xedit-core as a standalone editor model, without dragging in the rest
of the workspace.

## Adding a New Seam

When introducing a new subsystem boundary:

1. Define the trait in the crate that *consumes* it (not the one that
   implements it)
2. Provide a `No*` default implementation that returns a sensible
   failure code
3. Wire the real implementation in cms-machine, where all crates meet
4. Gate the dependency behind a Cargo feature if it's heavyweight
