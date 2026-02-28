use crate::error::{IucvError, Result};
use crate::handler::{MachineContext, MachineHandler};
use crate::machine_id::MachineId;
use crate::message::SmsgMessage;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};

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
    fn drop(&mut self) {
        // 1. Drop the router sender so the router loop exits naturally.
        //    Use try_lock to avoid panicking if already locked (e.g. during shutdown).
        if let Ok(mut guard) = self.router_tx.try_lock() {
            guard.take();
        }
        // 2. Abort the router task first — this releases any read lock it may
        //    hold on `machines`, ensuring try_write() below can succeed.
        if let Ok(mut guard) = self.router_task.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        // 3. Abort all machine tasks so they don't leak.
        //    try_write() can fail if the router task (now aborted) held a read
        //    lock at the instant Drop ran. Call shutdown().await before drop to
        //    guarantee clean teardown.
        match self.machines.try_write() {
            Ok(mut machines) => {
                for (_key, entry) in machines.drain() {
                    entry.task_handle.abort();
                }
            }
            Err(_) => {
                eprintln!(
                    "DMSIUC032W Supervisor::drop could not acquire write lock; \
                     machine tasks may leak. Call shutdown().await before drop."
                );
            }
        }
    }
}

impl Default for Supervisor {
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
        let router_tx = {
            let guard = self.router_tx.lock().await;
            guard.clone().ok_or(IucvError::SupervisorDown)?
        };

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
    /// The machine is first removed from the registry (preventing new messages
    /// from being routed to it), then the `Logoff` signal is sent. Since no
    /// new messages can arrive after removal, `send().await` is safe — the
    /// channel can only drain, so it cannot deadlock.
    pub async fn logoff(&self, id: &MachineId) -> Result<()> {
        let entry = {
            let mut machines = self.machines.write().await;
            machines
                .remove(id)
                .ok_or_else(|| IucvError::AlreadyLoggedOff(id.as_str().to_string()))?
        };

        // Machine is removed from the registry; the router will no longer
        // route messages to its channel. send().await is safe — the channel
        // can only drain from here.
        let _ = entry.signal_tx.send(MachineSignal::Logoff).await;
        // Wait for the machine task to complete so on_logoff() finishes.
        entry
            .task_handle
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
        let signal_tx = {
            let machines = self.machines.read().await;
            // Validate sender is registered.
            if !machines.contains_key(from) {
                return Err(IucvError::MachineNotFound(from.as_str().to_string()));
            }
            let entry = machines
                .get(to)
                .ok_or_else(|| IucvError::MachineNotFound(to.as_str().to_string()))?;
            entry.signal_tx.clone()
        }; // read guard dropped

        let msg = SmsgMessage::new(from.clone(), to.clone(), text)?;

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
    /// Phase 1: sends logoff signals to all machines concurrently via
    /// `try_send` — a machine with a full channel cannot delay others.
    ///
    /// Phase 2: drops all signal senders and joins all task handles.
    /// Dropping the sender before awaiting ensures that even if `try_send`
    /// failed (channel full), the machine exits via `recv() → None` rather
    /// than deadlocking. If the Logoff signal was not enqueued, `on_logoff`
    /// will not be called for that machine.
    pub async fn shutdown(&self) {
        // Drain all machine entries.
        let entries: Vec<(MachineId, MachineEntry)> = {
            let mut machines = self.machines.write().await;
            machines.drain().collect()
        };

        // Phase 1: non-blocking logoff signals to all machines.
        for (_key, entry) in &entries {
            let _ = entry.signal_tx.try_send(MachineSignal::Logoff);
        }

        // Phase 2: drop senders and join tasks. Destructuring ensures
        // signal_tx is dropped before awaiting task_handle, preventing
        // deadlock when the Logoff signal was not enqueued (channel full).
        for (key, entry) in entries {
            let MachineEntry {
                signal_tx,
                task_handle,
            } = entry;
            drop(signal_tx);
            if let Err(e) = task_handle.await {
                eprintln!("DMSIUC028W Machine {} panicked during shutdown: {}", key, e);
            }
        }

        // Drop the router sender so the router loop exits.
        {
            let mut guard = self.router_tx.lock().await;
            guard.take();
        }

        // Wait for the router task to complete.
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
            MachineSignal::Logoff => {
                handler.on_logoff(&ctx);
                break;
            }
        }
    }
}

/// The router task: receives outbound messages from machine contexts and
/// dispatches them to the target machine's signal channel.
///
/// Uses `try_send` so a single machine with a full signal channel cannot
/// stall delivery to all other machines (head-of-line blocking). Messages
/// to a machine with a full inbox are silently dropped — consistent with
/// fire-and-forget SMSG semantics.
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
