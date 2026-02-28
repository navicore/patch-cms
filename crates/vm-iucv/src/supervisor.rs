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
    machines: Arc<RwLock<HashMap<String, MachineEntry>>>,
    router_tx: Mutex<Option<mpsc::Sender<SmsgMessage>>>,
    router_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
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
        let key = id.as_str().to_string();
        if machines.contains_key(&key) {
            return Err(IucvError::AlreadyRunning(key));
        }

        let (signal_tx, signal_rx) = mpsc::channel::<MachineSignal>(64);
        let ctx = MachineContext::new(id.clone(), router_tx);
        let task_handle = tokio::spawn(run_machine(handler, ctx, signal_rx));
        machines.insert(
            key,
            MachineEntry {
                signal_tx,
                task_handle,
            },
        );

        Ok(())
    }

    /// Log off (shut down) a running machine.
    pub async fn logoff(&self, id: &MachineId) -> Result<()> {
        let entry = {
            let mut machines = self.machines.write().await;
            let key = id.as_str().to_string();
            machines
                .remove(&key)
                .ok_or(IucvError::AlreadyLoggedOff(key))?
        };

        // Send logoff signal; ignore error if task already exited.
        let _ = entry.signal_tx.send(MachineSignal::Logoff).await;
        // Wait for the machine task to complete so on_logoff() finishes.
        let _ = entry.task_handle.await;
        Ok(())
    }

    /// Send an SMSG from one machine to another.
    pub async fn smsg(&self, from: &MachineId, to: &MachineId, text: &str) -> Result<()> {
        // Clone the sender and drop the read guard before awaiting send.
        let signal_tx = {
            let machines = self.machines.read().await;
            let key = to.as_str().to_string();
            let entry = machines.get(&key).ok_or(IucvError::MachineNotFound(key))?;
            entry.signal_tx.clone()
        };

        let msg = SmsgMessage {
            from: from.clone(),
            to: to.clone(),
            text: text.to_string(),
        };

        signal_tx
            .send(MachineSignal::Smsg(msg))
            .await
            .map_err(|_| IucvError::DeliveryFailed(to.as_str().to_string()))
    }

    /// Return a sorted list of all running machine ids (CP QUERY NAMES).
    pub async fn query_names(&self) -> Vec<MachineId> {
        let machines = self.machines.read().await;
        let mut names: Vec<MachineId> = machines
            .keys()
            .map(|k| MachineId::new(k).expect("supervisor key is always a valid MachineId"))
            .collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        names
    }

    /// Shut down all machines and the router task.
    ///
    /// Sends logoff signals to every running machine, joins all machine tasks
    /// (ensuring `on_logoff()` completes), then shuts down the router.
    pub async fn shutdown(&self) {
        // Drain all machine entries.
        let entries: Vec<(String, MachineEntry)> = {
            let mut machines = self.machines.write().await;
            machines.drain().collect()
        };

        // Send logoff signals and join all machine tasks.
        for (_key, entry) in entries {
            let _ = entry.signal_tx.send(MachineSignal::Logoff).await;
            let _ = entry.task_handle.await;
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
async fn router_loop(
    mut rx: mpsc::Receiver<SmsgMessage>,
    machines: Arc<RwLock<HashMap<String, MachineEntry>>>,
) {
    while let Some(msg) = rx.recv().await {
        // Clone the sender and drop the read guard before awaiting send,
        // so a slow consumer cannot block the entire router.
        let signal_tx = {
            let machines = machines.read().await;
            let key = msg.to.as_str().to_string();
            machines.get(&key).map(|e| e.signal_tx.clone())
        };
        if let Some(tx) = signal_tx {
            let _ = tx.send(MachineSignal::Smsg(msg)).await;
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
        assert_eq!(msgs[0].from.as_str(), "ALICE");
        assert_eq!(msgs[0].text, "Hello Bob");

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
    async fn machine_to_machine_smsg_via_context() {
        // A handler that auto-replies to any SMSG using try_send_smsg.
        struct EchoHandler;
        impl MachineHandler for EchoHandler {
            fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
                let text = format!("ECHO: {}", msg.text);
                let _ = ctx.try_send_smsg(&msg.from, &text);
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
        assert_eq!(msgs[0].from.as_str(), "BOB");
        assert!(msgs[0].text.contains("ECHO: Ping"));

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
}
