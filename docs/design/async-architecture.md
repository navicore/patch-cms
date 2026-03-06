# Async Architecture

## The Model

VM/CMS is inherently concurrent: multiple virtual machines run
independently, each processing commands sequentially, while exchanging
messages asynchronously. We model this with Tokio's task-per-machine
actor pattern.

## Supervisor and Machine Tasks

The `Supervisor` (in vm-iucv) is the "Control Program." It manages
machine lifecycles and message routing.

```
Supervisor
├── machines: Arc<RwLock<HashMap<MachineId, MachineEntry>>>
├── router_task      -- routes SMSG between machines
├── path_cmd_task    -- handles IUCV path lifecycle
└── per-machine tasks:
    └── run_machine(handler, ctx, signal_rx)
```

Each machine is a Tokio task running a simple receive loop:

```rust
async fn run_machine(mut handler, ctx, mut signal_rx) {
    handler.on_ipl(&ctx);

    while let Some(signal) = signal_rx.recv().await {
        match signal {
            MachineSignal::Smsg(msg) => handler.on_smsg(&ctx, msg),
            MachineSignal::Logoff   => break,
            // ... IUCV signals ...
        }
    }

    handler.on_logoff(&ctx);
}
```

**Key decision: synchronous callbacks on async tasks.** The
`MachineHandler` trait methods are synchronous (`&mut self`, not
`async`). This means:

- Handlers cannot `.await` inside callbacks
- Handler state is never shared across tasks (no `Send + Sync` bound
  on fields)
- The machine task yields to the runtime only between signals

This matches real CMS semantics -- a virtual machine processes one
event at a time -- and keeps handler implementations simple.

## Message Routing

All inter-machine communication flows through typed channels:

```
Machine A                          Machine B
    |                                  |
    | ctx.try_send_smsg("B", text)     |
    |                                  |
    v                                  |
 smsg_tx ──> router_task ──> signal_tx ──> signal_rx
                                       |
                                       v
                                  on_smsg(msg)
```

The router uses `try_send` (non-blocking) to dispatch. If a machine's
signal channel is full, the message is dropped. This is fire-and-forget
by design -- it prevents one slow machine from blocking the entire
system.

## The Sync-Async Bridge (Console)

The CMS interactive console poses a challenge: `stdin` is blocking I/O,
but the machine handler runs on an async task. The bridge works like
this:

```
[blocking thread]           [async task]
     stdin                       |
       |                         |
  read line                      |
       |                         |
  cmd_tx.send(line) ──────> drain_commands()
       |                    (in on_smsg callback)
  $CON SMSG wakes machine       |
       |                    execute command
  wait for BATCH_DONE           |
       |                    output_tx.send(lines)
  print output <────────── BATCH_DONE sentinel
```

1. The console thread reads stdin and sends commands via
   `std::sync::mpsc` (not Tokio's -- it runs on a blocking thread)
2. It wakes the machine by sending an SMSG from the `$CON`
   pseudo-machine
3. The handler's `on_smsg` callback drains the command channel and
   executes commands synchronously
4. Output lines flow back via a channel, terminated by a `BATCH_DONE`
   sentinel
5. The console thread collects output until it sees the sentinel

The sentinel-based protocol avoids timeouts and polling. It is
deterministic: the console knows exactly when the machine is done.

## Why Not Async Handlers?

We considered making `MachineHandler` methods async. Reasons we didn't:

1. **CMS is sequential.** A real CMS machine never processes two
   commands concurrently. Async handlers would add complexity for a
   capability we don't want.

2. **Handler state stays simple.** With sync callbacks, handler fields
   can be plain `Vec`, `HashMap`, etc. Async would require
   `Send + Sync` bounds or `spawn_local`, adding friction for every
   handler implementation.

3. **Outbound messaging is already async.** `ctx.try_send_smsg()`
   enqueues to a channel that the router processes asynchronously.
   The handler doesn't need to await the delivery.

4. **Blocking work is bounded.** CMS commands (LISTFILE, CHANGE, etc.)
   are fast in-memory operations. There is no disk I/O or network
   call that would benefit from yielding mid-command.

If a future use case requires long-running async work inside a handler,
the recommended pattern is: spawn a separate Tokio task and
communicate results back via SMSG.

## Tokio Usage

- **Runtime:** `rt-multi-thread` in cms-machine's main binary;
  vm-iucv itself only requires `rt` and `sync`
- **Channels:** `tokio::sync::mpsc` for machine signals and router
  queues; `std::sync::mpsc` for the blocking console bridge
- **Locks:** `tokio::sync::RwLock` for the machine registry (read-heavy,
  rarely written)
- **No timers in the core.** Timeouts exist only as safety nets in
  the console bridge, not as part of the actor protocol
