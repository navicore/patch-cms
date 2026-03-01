use crate::error::{IucvError, Result};
use crate::handler::{MachineContext, MachineHandler};
use crate::machine_id::MachineId;
use crate::message::SmsgMessage;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinSet;

/// Signal sent to a running machine task.
enum MachineSignal {
    Smsg(SmsgMessage),
    Logoff,
}

/// Entry for a running machine in the supervisor's registry.
struct MachineEntry {
    signal_tx: mpsc::Sender<MachineSignal>,
    task_handle: tokio::task::JoinHandle<()>,
}

/// The CP (Control Program) — manages all running machines.
///
/// Each machine is a Tokio task with its own handler and signal channel.
/// A background router task dispatches outbound messages from machine contexts
/// to the appropriate target machine's signal channel.
///
/// # Panics
///
/// `Supervisor::new()` and `Supervisor::default()` call `tokio::spawn` and
/// will panic if invoked outside a Tokio runtime.
pub struct Supervisor {
    machines: Arc<RwLock<HashMap<MachineId, MachineEntry>>>,
    router_tx: Mutex<Option<mpsc::Sender<SmsgMessage>>>,
    router_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for Supervisor {
    /// Best-effort cleanup. `abort()` only schedules cancellation — it does
    /// not synchronously release locks held by aborted tasks. If the router
    /// task holds a read lock on `machines` at the instant `Drop` runs,
    /// `try_write()` will fail and machine tasks will leak.
    ///
    /// **Always call [`shutdown().await`](Supervisor::shutdown) before dropping
    /// the `Supervisor`** to guarantee clean teardown.
    fn drop(&mut self) {
        // 1. Drop the router sender so the router loop exits naturally.
        if let Ok(mut guard) = self.router_tx.try_lock() {
            guard.take();
        }
        // 2. Abort the router task. Note: abort() is non-blocking — the task's
        //    lock guard is released only after the scheduler polls and drops
        //    the future, which may not happen before step 3.
        if let Ok(mut guard) = self.router_task.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        // 3. Abort all machine tasks. try_write() may fail if the router task
        //    still holds a read lock (abort is async). This is inherently
        //    best-effort — use shutdown().await for guaranteed cleanup.
        if let Ok(mut machines) = self.machines.try_write() {
            for (_key, entry) in machines.drain() {
                entry.task_handle.abort();
            }
        }
    }
}

impl Default for Supervisor {
    /// Creates a new `Supervisor` via [`Supervisor::new`].
    ///
    /// # Panics
    ///
    /// Panics if called outside a Tokio runtime.
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    /// Create a new supervisor and spawn its background router task.
    ///
    /// # Panics
    ///
    /// Panics if called outside a Tokio runtime.
    pub fn new() -> Self {
        let machines = Arc::new(RwLock::new(HashMap::new()));
        let (router_tx, router_rx) = mpsc::channel::<SmsgMessage>(256);

        let router_machines = Arc::clone(&machines);
        let router_handle = tokio::spawn(router_loop(router_rx, router_machines));

        Supervisor {
            machines,
            router_tx: Mutex::new(Some(router_tx)),
            router_task: Mutex::new(Some(router_handle)),
        }
    }

    /// IPL (boot) a machine with the given handler.
    pub async fn ipl(&self, id: &MachineId, handler: impl MachineHandler) -> Result<()> {
        // Hold router_tx lock across the entire operation so a concurrent
        // shutdown() cannot complete between the router check and the machine
        // insertion (which would create a zombie machine with a stale sender).
        let guard = self.router_tx.lock().await;
        let router_tx = guard.as_ref().ok_or(IucvError::SupervisorDown)?.clone();

        let mut machines = self.machines.write().await;
        if machines.contains_key(id) {
            return Err(IucvError::AlreadyRunning(id.as_str().to_string()));
        }

        let (signal_tx, signal_rx) = mpsc::channel::<MachineSignal>(64);
        let ctx = MachineContext::new(id.clone(), router_tx);
        let task_handle = tokio::spawn(run_machine(handler, ctx, signal_rx));
        machines.insert(
            id.clone(),
            MachineEntry {
                signal_tx,
                task_handle,
            },
        );

        Ok(())
    }

    /// Log off (shut down) a running machine.
    ///
    /// The machine is removed from the registry, then the `Logoff` signal is
    /// delivered via `try_send`. The signal sender is dropped immediately
    /// after, ensuring the machine task exits even if the signal channel was
    /// full (via `recv() → None`).
    ///
    /// **Note:** `task_handle.await` blocks until the machine task completes.
    /// If a handler callback (`on_smsg`, `on_ipl`) blocks indefinitely, this
    /// method will also block. Handlers must return promptly from callbacks.
    pub async fn logoff(&self, id: &MachineId) -> Result<()> {
        let entry = {
            let mut machines = self.machines.write().await;
            machines
                .remove(id)
                .ok_or_else(|| IucvError::AlreadyLoggedOff(id.as_str().to_string()))?
        };

        let MachineEntry {
            signal_tx,
            task_handle,
        } = entry;

        // try_send + drop: matching shutdown()'s pattern. The drop closes the
        // channel, guaranteeing the task exits via recv() → None if the Logoff
        // signal could not be enqueued.
        let _ = signal_tx.try_send(MachineSignal::Logoff);
        drop(signal_tx);

        task_handle
            .await
            .map_err(|e| IucvError::MachinePanicked(format!("{e}")))?;
        Ok(())
    }

    /// Send an SMSG from one machine to another.
    ///
    /// Uses non-blocking `try_send` for consistent fire-and-forget semantics
    /// with the router path (via [`MachineContext::try_send_smsg`]). Returns
    /// [`IucvError::ChannelBusy`] if the target machine's signal channel is
    /// at capacity — the caller may retry.
    ///
    /// The `from` identity is checked for registration but not for caller
    /// authenticity — any caller with a valid registered `MachineId` can
    /// specify it as `from`. Messages sent through [`MachineContext`]
    /// always carry the machine's true identity set at IPL time.
    ///
    /// Note: if the target machine logs off between the registry lookup and
    /// the `try_send`, `DeliveryFailed` (RC=16) is returned rather than
    /// `MachineNotFound` (RC=12). Callers should treat both as
    /// "target unavailable."
    pub async fn smsg(&self, from: &MachineId, to: &MachineId, text: &str) -> Result<()> {
        // Build the message and clone the sender under the read lock so
        // the sender identity is validated while we know it is registered.
        let (signal_tx, msg) = {
            let machines = self.machines.read().await;
            // Validate sender is registered.
            if !machines.contains_key(from) {
                return Err(IucvError::MachineNotFound(from.as_str().to_string()));
            }
            let entry = machines
                .get(to)
                .ok_or_else(|| IucvError::MachineNotFound(to.as_str().to_string()))?;
            let msg = SmsgMessage::new(from.clone(), to.clone(), text)?;
            (entry.signal_tx.clone(), msg)
        }; // read guard dropped

        signal_tx
            .try_send(MachineSignal::Smsg(msg))
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => {
                    IucvError::ChannelBusy(to.as_str().to_string())
                }
                mpsc::error::TrySendError::Closed(_) => {
                    IucvError::DeliveryFailed(to.as_str().to_string())
                }
            })
    }

    /// Return a sorted list of all running machine ids (CP QUERY NAMES).
    pub async fn query_names(&self) -> Vec<MachineId> {
        let machines = self.machines.read().await;
        let mut names: Vec<MachineId> = machines.keys().cloned().collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        names
    }

    /// Shut down all machines and the router task.
    ///
    /// Acquires `router_tx` then `machines` — matching the lock order in
    /// [`ipl`](Self::ipl) — to prevent a TOCTOU race where `ipl()` inserts
    /// a zombie machine between `shutdown()`'s drain and router teardown.
    ///
    /// Phase 1: sends logoff signals to all machines via `try_send` and drops
    /// all signal senders — a machine with a full channel cannot delay others.
    /// Dropping the sender ensures the machine task exits via `recv() → None`
    /// even if `try_send` failed (channel full).
    ///
    /// Phase 2: joins all task handles concurrently via `JoinSet` so that
    /// shutdown latency is `O(max on_logoff time)` rather than `O(sum)`.
    pub async fn shutdown(&self) {
        // Acquire router_tx first (matching ipl() lock order), then machines.
        // Taking router_tx atomically with the drain prevents ipl() from
        // cloning a live sender while we are shutting down.
        let entries: Vec<(MachineId, MachineEntry)> = {
            let mut router_guard = self.router_tx.lock().await;
            let mut machines = self.machines.write().await;
            router_guard.take(); // mark shut down
            machines.drain().collect()
        };

        // Phase 1: non-blocking logoff signals, then drop all senders.
        let mut join_set = JoinSet::new();
        for (key, entry) in entries {
            let MachineEntry {
                signal_tx,
                task_handle,
            } = entry;
            let _ = signal_tx.try_send(MachineSignal::Logoff);
            drop(signal_tx);
            join_set.spawn(async move { (key, task_handle.await) });
        }

        // Phase 2: join all machine tasks concurrently.
        while let Some(result) = join_set.join_next().await {
            if let Ok((key, Err(e))) = result {
                eprintln!("DMSIUC028E Machine {} panicked during shutdown: {}", key, e);
            }
        }

        // Wait for the router task to complete (router_tx was already taken above).
        let handle = {
            let mut guard = self.router_task.lock().await;
            guard.take()
        };
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }
}

/// The machine task loop: calls lifecycle hooks and processes signals.
///
/// `on_logoff` is called when the machine receives an explicit `Logoff`
/// signal or the channel closes (e.g. during `shutdown()` when `try_send`
/// failed and the sender was dropped). **However, if `on_ipl` or `on_smsg`
/// panics, the task unwinds and `on_logoff` is NOT called.** Handlers that
/// require guaranteed cleanup should use RAII guards rather than relying
/// on `on_logoff`.
///
/// Note: `Smsg` signals that were in-flight in the router when `Logoff` is
/// enqueued may or may not be delivered, depending on channel ordering.
/// The `MachineHandler` trait documents this race.
async fn run_machine(
    mut handler: impl MachineHandler,
    ctx: MachineContext,
    mut signal_rx: mpsc::Receiver<MachineSignal>,
) {
    handler.on_ipl(&ctx);

    while let Some(signal) = signal_rx.recv().await {
        match signal {
            MachineSignal::Smsg(msg) => handler.on_smsg(&ctx, msg),
            MachineSignal::Logoff => break,
        }
    }

    // Called on normal exit (Logoff signal or channel close).
    // NOT called if on_ipl or on_smsg panicked — the task unwinds past this point.
    handler.on_logoff(&ctx);
}

/// The router task: receives outbound messages from machine contexts and
/// dispatches them to the target machine's signal channel.
///
/// Uses `try_send` so a single machine with a full signal channel cannot
/// stall delivery to all other machines (head-of-line blocking). Messages
/// to a machine with a full inbox are silently dropped — consistent with
/// fire-and-forget SMSG semantics.
///
/// **Performance note:** acquires a read lock on `machines` per message.
/// Under high throughput this creates read-side contention that may delay
/// `ipl()`/`logoff()` write-lock acquisitions (Tokio's `RwLock` is fair).
/// Acceptable for moderate machine counts; a lock-free routing table could
/// replace this if high message rates are needed.
async fn router_loop(
    mut rx: mpsc::Receiver<SmsgMessage>,
    machines: Arc<RwLock<HashMap<MachineId, MachineEntry>>>,
) {
    while let Some(msg) = rx.recv().await {
        let machines = machines.read().await;
        if let Some(entry) = machines.get(msg.to()) {
            // Non-blocking: a full target channel drops the message rather
            // than stalling the router for every other machine.
            let _ = entry.signal_tx.try_send(MachineSignal::Smsg(msg));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::collector;
    use std::time::Duration;

    /// Poll a condition with timeout instead of a fixed sleep.
    async fn wait_for(timeout_ms: u64, mut condition: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        while tokio::time::Instant::now() < deadline {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("wait_for timed out after {}ms", timeout_ms);
    }

    #[tokio::test]
    async fn ipl_and_query_names() {
        let sup = Supervisor::new();
        let (handler, _handle) = collector();
        let id = MachineId::new("ALICE").unwrap();
        sup.ipl(&id, handler).await.unwrap();

        let names = sup.query_names().await;
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].as_str(), "ALICE");

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn duplicate_ipl_error() {
        let sup = Supervisor::new();
        let id = MachineId::new("ALICE").unwrap();

        let (h1, _) = collector();
        sup.ipl(&id, h1).await.unwrap();

        let (h2, _) = collector();
        let err = sup.ipl(&id, h2).await.unwrap_err();
        assert_eq!(err.rc(), 8);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn logoff_removes_machine() {
        let sup = Supervisor::new();
        let id = MachineId::new("ALICE").unwrap();

        let (handler, _handle) = collector();
        sup.ipl(&id, handler).await.unwrap();
        assert_eq!(sup.query_names().await.len(), 1);

        // logoff() joins the machine task, so no settle needed.
        sup.logoff(&id).await.unwrap();
        assert!(sup.query_names().await.is_empty());

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn logoff_unknown_error() {
        let sup = Supervisor::new();
        let id = MachineId::new("GHOST").unwrap();

        let err = sup.logoff(&id).await.unwrap_err();
        assert_eq!(err.rc(), 12);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn smsg_delivery() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        sup.smsg(&alice, &bob, "Hello Bob").await.unwrap();
        wait_for(2000, || bob_handle.count() >= 1).await;

        assert_eq!(bob_handle.count(), 1);
        let msgs = bob_handle.messages();
        assert_eq!(msgs[0].from().as_str(), "ALICE");
        assert_eq!(msgs[0].text(), "Hello Bob");

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn smsg_unknown_error() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let ghost = MachineId::new("GHOST").unwrap();

        let (handler, _) = collector();
        sup.ipl(&alice, handler).await.unwrap();

        let err = sup.smsg(&alice, &ghost, "Hello").await.unwrap_err();
        assert_eq!(err.rc(), 12);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn smsg_unregistered_sender_error() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();
        let ghost = MachineId::new("GHOST").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        // GHOST is not registered — smsg should reject the forged sender.
        let err = sup.smsg(&ghost, &bob, "Forged").await.unwrap_err();
        assert_eq!(err.rc(), 12);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn machine_to_machine_smsg_via_context() {
        // A handler that auto-replies to any SMSG using try_send_smsg.
        struct EchoHandler;
        impl MachineHandler for EchoHandler {
            fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
                let text = format!("ECHO: {}", msg.text());
                let _ = ctx.try_send_smsg(msg.from(), &text);
            }
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, alice_handle) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, EchoHandler).await.unwrap();

        // Alice sends to Bob, Bob auto-replies via context.
        sup.smsg(&alice, &bob, "Ping").await.unwrap();
        wait_for(2000, || alice_handle.count() >= 1).await;

        // Alice should have received the echo.
        assert_eq!(alice_handle.count(), 1);
        let msgs = alice_handle.messages();
        assert_eq!(msgs[0].from().as_str(), "BOB");
        assert!(msgs[0].text().contains("ECHO: Ping"));

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn smsg_during_on_ipl() {
        // A handler that sends an SMSG to a peer during on_ipl.
        struct GreeterHandler {
            target: MachineId,
        }
        impl MachineHandler for GreeterHandler {
            fn on_ipl(&mut self, ctx: &MachineContext) {
                let _ = ctx.try_send_smsg(&self.target, "Booted!");
            }
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        // IPL Alice first (collector), then Bob who greets Alice on boot.
        let (h_alice, alice_handle) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();

        let greeter = GreeterHandler {
            target: alice.clone(),
        };
        sup.ipl(&bob, greeter).await.unwrap();

        wait_for(2000, || alice_handle.count() >= 1).await;
        let msgs = alice_handle.messages();
        assert_eq!(msgs[0].from().as_str(), "BOB");
        assert_eq!(msgs[0].text(), "Booted!");

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn ipl_panic_surfaces_on_logoff() {
        struct PanicOnIpl;
        impl MachineHandler for PanicOnIpl {
            fn on_ipl(&mut self, _ctx: &MachineContext) {
                panic!("intentional on_ipl panic");
            }
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
        }

        let sup = Supervisor::new();
        let id = MachineId::new("CRASHER").unwrap();
        sup.ipl(&id, PanicOnIpl).await.unwrap();

        // The task panicked in on_ipl, so logoff should surface MachinePanicked.
        let err = sup.logoff(&id).await.unwrap_err();
        assert_eq!(err.rc(), 28);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn smsg_panic_surfaces_on_logoff() {
        struct PanicOnSmsg;
        impl MachineHandler for PanicOnSmsg {
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {
                panic!("intentional on_smsg panic");
            }
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, PanicOnSmsg).await.unwrap();

        // Send a message to trigger the panic in on_smsg.
        sup.smsg(&alice, &bob, "trigger panic").await.unwrap();

        // logoff() joins the task handle — the panic surfaces as MachinePanicked.
        let err = sup.logoff(&bob).await.unwrap_err();
        assert_eq!(err.rc(), 28);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn logoff_surfaces_handler_panic() {
        struct PanicOnLogoff;
        impl MachineHandler for PanicOnLogoff {
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
            fn on_logoff(&mut self, _ctx: &MachineContext) {
                panic!("intentional test panic");
            }
        }

        let sup = Supervisor::new();
        let id = MachineId::new("CRASHER").unwrap();
        sup.ipl(&id, PanicOnLogoff).await.unwrap();

        let err = sup.logoff(&id).await.unwrap_err();
        assert_eq!(err.rc(), 28);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_all() {
        let sup = Supervisor::new();

        let (h1, _) = collector();
        let (h2, _) = collector();
        sup.ipl(&MachineId::new("A").unwrap(), h1).await.unwrap();
        sup.ipl(&MachineId::new("B").unwrap(), h2).await.unwrap();
        assert_eq!(sup.query_names().await.len(), 2);

        sup.shutdown().await;
        assert!(sup.query_names().await.is_empty());
    }

    #[tokio::test]
    async fn shutdown_calls_on_logoff() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let called = Arc::new(AtomicBool::new(false));

        struct TrackLogoff(Arc<AtomicBool>);
        impl MachineHandler for TrackLogoff {
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
            fn on_logoff(&mut self, _ctx: &MachineContext) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let sup = Supervisor::new();
        sup.ipl(
            &MachineId::new("A").unwrap(),
            TrackLogoff(Arc::clone(&called)),
        )
        .await
        .unwrap();

        sup.shutdown().await;
        assert!(
            called.load(Ordering::SeqCst),
            "on_logoff must be called during shutdown"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn smsg_channel_busy() {
        // A handler that signals when it enters on_ipl, then blocks until
        // released. The rendezvous channel replaces a timing-based sleep.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();

        struct IplBlocker {
            ready: std::sync::mpsc::Sender<()>,
            release: std::sync::mpsc::Receiver<()>,
        }
        impl MachineHandler for IplBlocker {
            fn on_ipl(&mut self, _ctx: &MachineContext) {
                let _ = self.ready.send(()); // signal: "I'm in on_ipl"
                let _ = self.release.recv(); // block until released
            }
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(
            &bob,
            IplBlocker {
                ready: ready_tx,
                release: release_rx,
            },
        )
        .await
        .unwrap();

        // Wait for bob's task to reach on_ipl (deterministic, no sleep).
        ready_rx.recv().unwrap();

        // Fill all 64 slots in bob's signal channel.
        for i in 0..64 {
            sup.smsg(&alice, &bob, &format!("fill-{i}")).await.unwrap();
        }

        // The 65th message should get ChannelBusy (RC=20).
        let err = sup.smsg(&alice, &bob, "overflow").await.unwrap_err();
        assert_eq!(err.rc(), 20);

        // Release on_ipl so shutdown can complete cleanly.
        drop(release_tx);
        sup.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn router_drops_to_full_channel() {
        // Bob signals when in on_ipl, then blocks until released.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();

        struct IplBlocker {
            ready: std::sync::mpsc::Sender<()>,
            release: std::sync::mpsc::Receiver<()>,
        }
        impl MachineHandler for IplBlocker {
            fn on_ipl(&mut self, _ctx: &MachineContext) {
                let _ = self.ready.send(());
                let _ = self.release.recv();
            }
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
        }

        // Alice sends messages to bob via the router during on_ipl.
        struct Sender {
            target: MachineId,
        }
        impl MachineHandler for Sender {
            fn on_ipl(&mut self, ctx: &MachineContext) {
                for i in 0..10 {
                    let _ = ctx.try_send_smsg(&self.target, &format!("msg-{i}"));
                }
            }
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        sup.ipl(
            &bob,
            IplBlocker {
                ready: ready_tx,
                release: release_rx,
            },
        )
        .await
        .unwrap();

        // Wait for bob's task to reach on_ipl (deterministic).
        ready_rx.recv().unwrap();

        // Fill bob's 64-slot channel via the supervisor.
        for i in 0..64 {
            sup.smsg(&bob, &bob, &format!("fill-{i}")).await.unwrap();
        }

        // Alice sends via router — the router silently drops these since
        // bob's channel is full. No panic or hang should occur.
        sup.ipl(
            &alice,
            Sender {
                target: bob.clone(),
            },
        )
        .await
        .unwrap();

        // Give the router time to attempt delivery of alice's messages.
        tokio::time::sleep(Duration::from_millis(100)).await;

        drop(release_tx);
        sup.shutdown().await;
    }

    #[tokio::test]
    async fn smsg_text_too_long() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        let long_text = "x".repeat(237);
        let err = sup.smsg(&alice, &bob, &long_text).await.unwrap_err();
        assert_eq!(err.rc(), 24);

        sup.shutdown().await;
    }
}
