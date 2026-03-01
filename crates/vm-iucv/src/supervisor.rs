use crate::error::{IucvError, Result};
use crate::handler::{MachineContext, MachineHandler, PathCommand};
use crate::machine_id::MachineId;
use crate::message::SmsgMessage;
use crate::path::{IucvBuffer, PathId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::task::JoinSet;

/// Signal sent to a running machine task.
enum MachineSignal {
    Smsg(SmsgMessage),
    Logoff,
    ConnectionPending {
        path: PathId,
        from: MachineId,
        accept_tx: oneshot::Sender<bool>,
    },
    ConnectionComplete {
        path: PathId,
        peer: MachineId,
    },
    ConnectionSevered {
        path: PathId,
        peer: MachineId,
    },
    IucvData {
        path: PathId,
        data: IucvBuffer,
    },
}

/// State of an IUCV path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathState {
    Pending,
    Established,
}

/// Registry entry for an IUCV path.
struct PathEntry {
    id: PathId,
    machine_a: MachineId,
    machine_b: MachineId,
    state: PathState,
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
    paths: Arc<RwLock<HashMap<PathId, PathEntry>>>,
    next_path_id: Arc<AtomicU32>,
    router_tx: Mutex<Option<mpsc::Sender<SmsgMessage>>>,
    path_cmd_tx: Mutex<Option<mpsc::Sender<PathCommand>>>,
    router_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    path_cmd_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
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
        // 1. Drop the router and path_cmd senders so loops exit naturally.
        if let Ok(mut guard) = self.router_tx.try_lock() {
            guard.take();
        }
        if let Ok(mut guard) = self.path_cmd_tx.try_lock() {
            guard.take();
        }
        // 2. Abort background tasks.
        if let Ok(mut guard) = self.router_task.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        if let Ok(mut guard) = self.path_cmd_task.try_lock() {
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

/// Drop guard that removes a pending path entry if the `connect()` future is
/// cancelled (dropped at an `.await` point). Call `defuse()` on the normal path
/// to prevent cleanup.
struct PendingPathGuard {
    paths: Option<Arc<RwLock<HashMap<PathId, PathEntry>>>>,
    path_id: PathId,
}

impl PendingPathGuard {
    fn defuse(mut self) {
        self.paths.take();
    }
}

impl Drop for PendingPathGuard {
    fn drop(&mut self) {
        if let Some(paths) = self.paths.take() {
            // Scope the try_write borrow so it's dropped before the else branch.
            let removed = {
                if let Ok(mut guard) = paths.try_write() {
                    guard.remove(&self.path_id);
                    true
                } else {
                    false
                }
            };
            if !removed {
                // Lock is contended — spawn a background task to clean up
                // once the lock becomes available.
                let id = self.path_id;
                tokio::spawn(async move {
                    paths.write().await.remove(&id);
                });
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
        let paths = Arc::new(RwLock::new(HashMap::new()));
        let next_path_id = Arc::new(AtomicU32::new(1));
        let (router_tx, router_rx) = mpsc::channel::<SmsgMessage>(256);
        let (path_cmd_tx, path_cmd_rx) = mpsc::channel::<PathCommand>(256);

        let router_machines = Arc::clone(&machines);
        let router_handle = tokio::spawn(router_loop(router_rx, router_machines));

        let pcmd_machines = Arc::clone(&machines);
        let pcmd_paths = Arc::clone(&paths);
        let path_cmd_handle = tokio::spawn(path_cmd_loop(path_cmd_rx, pcmd_machines, pcmd_paths));

        Supervisor {
            machines,
            paths,
            next_path_id,
            router_tx: Mutex::new(Some(router_tx)),
            path_cmd_tx: Mutex::new(Some(path_cmd_tx)),
            router_task: Mutex::new(Some(router_handle)),
            path_cmd_task: Mutex::new(Some(path_cmd_handle)),
        }
    }

    /// IPL (boot) a machine with the given handler.
    pub async fn ipl(&self, id: &MachineId, handler: impl MachineHandler) -> Result<()> {
        // Hold router_tx lock across the entire operation so a concurrent
        // shutdown() cannot complete between the router check and the machine
        // insertion (which would create a zombie machine with a stale sender).
        let guard = self.router_tx.lock().await;
        let router_tx = guard.as_ref().ok_or(IucvError::SupervisorDown)?.clone();
        let pcmd_guard = self.path_cmd_tx.lock().await;
        let path_cmd_tx = pcmd_guard
            .as_ref()
            .ok_or(IucvError::SupervisorDown)?
            .clone();

        let mut machines = self.machines.write().await;
        if machines.contains_key(id) {
            return Err(IucvError::AlreadyRunning(id.as_str().to_string()));
        }

        let (signal_tx, signal_rx) = mpsc::channel::<MachineSignal>(64);
        let ctx = MachineContext::new(id.clone(), router_tx, path_cmd_tx);
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

        // Sever all paths involving this machine, notify peers.
        self.sever_paths_for_machine(id).await;

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

    /// Initiate an IUCV connection from one machine to another.
    ///
    /// Returns the `PathId`. The path starts in Pending state; the target's
    /// `on_connection_pending` callback decides whether to accept or refuse.
    /// If accepted, both sides receive `on_connection_complete`. If refused,
    /// returns `ConnectionRefused`.
    pub async fn connect(&self, from: &MachineId, to: &MachineId) -> Result<PathId> {
        // Validate both machines exist and get target's signal sender.
        let target_tx = {
            let machines = self.machines.read().await;
            if !machines.contains_key(from) {
                return Err(IucvError::MachineNotFound(from.as_str().to_string()));
            }
            let target = machines
                .get(to)
                .ok_or_else(|| IucvError::MachineNotFound(to.as_str().to_string()))?;
            target.signal_tx.clone()
        };

        // Generate a new PathId and create the pending entry.
        let path_id = PathId(self.next_path_id.fetch_add(1, Ordering::Relaxed));
        {
            let mut paths = self.paths.write().await;
            paths.insert(
                path_id,
                PathEntry {
                    id: path_id,
                    machine_a: from.clone(),
                    machine_b: to.clone(),
                    state: PathState::Pending,
                },
            );
        }

        // Drop guard: if this future is cancelled at any `.await` below,
        // remove the pending path entry so it doesn't leak.
        let paths_ref = Arc::clone(&self.paths);
        let cancel_guard = PendingPathGuard {
            paths: Some(paths_ref),
            path_id,
        };

        // Send ConnectionPending to the target with a oneshot for the response.
        let (accept_tx, accept_rx) = oneshot::channel();
        let signal = MachineSignal::ConnectionPending {
            path: path_id,
            from: from.clone(),
            accept_tx,
        };
        if let Err(e) = target_tx.try_send(signal) {
            // Guard drops here and cleans up the pending entry.
            return Err(match e {
                mpsc::error::TrySendError::Full(_) => {
                    IucvError::ChannelBusy(to.as_str().to_string())
                }
                mpsc::error::TrySendError::Closed(_) => {
                    IucvError::DeliveryFailed(to.as_str().to_string())
                }
            });
        }

        // Wait for the target's accept/refuse decision.
        let accepted = accept_rx.await.unwrap_or(false);

        if !accepted {
            // Guard drops here and cleans up the pending entry.
            return Err(IucvError::ConnectionRefused(to.as_str().to_string()));
        }

        // Transition to Established. Re-check machine existence in the same
        // critical section to close the TOCTOU window where logoff(to) could
        // run between the initial existence check and this transition.
        let established = {
            let machines = self.machines.read().await;
            let mut paths = self.paths.write().await;
            if !machines.contains_key(to) {
                // Target logged off during negotiation.
                false
            } else if let Some(entry) = paths.get_mut(&path_id) {
                entry.state = PathState::Established;
                true
            } else {
                // Entry removed by concurrent sever_paths_for_machine.
                false
            }
        };

        if !established {
            // Guard drops here and cleans up any remaining entry.
            return Err(IucvError::MachineNotFound(to.as_str().to_string()));
        }

        // Success — defuse the guard so the Established entry persists.
        cancel_guard.defuse();

        // Notify both sides (best-effort: may be dropped if channel full).
        let machines = self.machines.read().await;
        if let Some(entry) = machines.get(from) {
            let _ = entry.signal_tx.try_send(MachineSignal::ConnectionComplete {
                path: path_id,
                peer: to.clone(),
            });
        }
        if from != to {
            if let Some(entry) = machines.get(to) {
                let _ = entry.signal_tx.try_send(MachineSignal::ConnectionComplete {
                    path: path_id,
                    peer: from.clone(),
                });
            }
        }

        Ok(path_id)
    }

    /// Sever an existing path. Delivers `ConnectionSevered` to both sides.
    ///
    /// The path entry is removed after notifying both sides, so a subsequent
    /// `sever()` on the same path returns `PathNotFound`.
    pub async fn sever(&self, path: PathId) -> Result<()> {
        let (machine_a, machine_b) = {
            let mut paths = self.paths.write().await;
            let entry = paths
                .remove(&path)
                .ok_or(IucvError::PathNotFound(path.as_u32()))?;
            (entry.machine_a, entry.machine_b)
        };

        // Notify both sides (best-effort). Skip duplicate for self-connections.
        let machines = self.machines.read().await;
        if let Some(entry) = machines.get(&machine_a) {
            let _ = entry.signal_tx.try_send(MachineSignal::ConnectionSevered {
                path,
                peer: machine_b.clone(),
            });
        }
        if machine_a != machine_b {
            if let Some(entry) = machines.get(&machine_b) {
                let _ = entry.signal_tx.try_send(MachineSignal::ConnectionSevered {
                    path,
                    peer: machine_a.clone(),
                });
            }
        }

        Ok(())
    }

    /// Return all active paths (Pending or Established).
    pub async fn query_paths(&self) -> Vec<PathId> {
        let paths = self.paths.read().await;
        paths.keys().copied().collect()
    }

    /// Sever all paths involving a machine (called during logoff).
    ///
    /// Removes path entries after notifying peers to prevent leaks.
    async fn sever_paths_for_machine(&self, id: &MachineId) {
        let to_sever: Vec<(PathId, MachineId)> = {
            let mut paths = self.paths.write().await;
            let involved: Vec<PathId> = paths
                .iter()
                .filter(|(_, e)| e.machine_a == *id || e.machine_b == *id)
                .map(|(pid, _)| *pid)
                .collect();

            let mut severed = Vec::with_capacity(involved.len());
            for pid in involved {
                if let Some(entry) = paths.remove(&pid) {
                    let peer = if entry.machine_a == *id {
                        entry.machine_b
                    } else {
                        entry.machine_a
                    };
                    severed.push((pid, peer));
                }
            }
            severed
        };

        // Notify peers (best-effort).
        if !to_sever.is_empty() {
            let machines = self.machines.read().await;
            for (pid, peer) in &to_sever {
                if let Some(entry) = machines.get(peer) {
                    let _ = entry.signal_tx.try_send(MachineSignal::ConnectionSevered {
                        path: *pid,
                        peer: id.clone(),
                    });
                }
            }
        }
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
        // Sever all paths before draining machines so that running machine
        // tasks receive ConnectionSevered signals (consistent with logoff).
        {
            let machines = self.machines.read().await;
            let mut paths = self.paths.write().await;
            for entry in paths.values() {
                // Notify machine_a.
                if let Some(m) = machines.get(&entry.machine_a) {
                    let _ = m.signal_tx.try_send(MachineSignal::ConnectionSevered {
                        path: entry.id,
                        peer: entry.machine_b.clone(),
                    });
                }
                // Notify machine_b (skip duplicate for self-connections).
                if entry.machine_a != entry.machine_b {
                    if let Some(m) = machines.get(&entry.machine_b) {
                        let _ = m.signal_tx.try_send(MachineSignal::ConnectionSevered {
                            path: entry.id,
                            peer: entry.machine_a.clone(),
                        });
                    }
                }
            }
            paths.clear();
        }

        // Acquire router_tx first (matching ipl() lock order), then machines.
        // Taking router_tx atomically with the drain prevents ipl() from
        // cloning a live sender while we are shutting down.
        let entries: Vec<(MachineId, MachineEntry)> = {
            let mut router_guard = self.router_tx.lock().await;
            let mut pcmd_guard = self.path_cmd_tx.lock().await;
            let mut machines = self.machines.write().await;
            router_guard.take(); // mark shut down
            pcmd_guard.take();
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

        // Wait for background tasks to complete.
        let router_handle = {
            let mut guard = self.router_task.lock().await;
            guard.take()
        };
        if let Some(handle) = router_handle {
            let _ = handle.await;
        }
        let pcmd_handle = {
            let mut guard = self.path_cmd_task.lock().await;
            guard.take()
        };
        if let Some(handle) = pcmd_handle {
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
            MachineSignal::ConnectionPending {
                path,
                from,
                accept_tx,
            } => {
                let accepted = handler.on_connection_pending(&ctx, path, &from);
                let _ = accept_tx.send(accepted);
            }
            MachineSignal::ConnectionComplete { path, peer } => {
                handler.on_connection_complete(&ctx, path, &peer);
            }
            MachineSignal::ConnectionSevered { path, peer } => {
                handler.on_connection_severed(&ctx, path, &peer);
            }
            MachineSignal::IucvData { path, data } => {
                handler.on_iucv_data(&ctx, path, data);
            }
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

/// Background task processing path commands from machine contexts.
async fn path_cmd_loop(
    mut rx: mpsc::Receiver<PathCommand>,
    machines: Arc<RwLock<HashMap<MachineId, MachineEntry>>>,
    paths: Arc<RwLock<HashMap<PathId, PathEntry>>>,
) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            PathCommand::Sever { path, from } => {
                let notify = {
                    let mut paths = paths.write().await;
                    if let Some(entry) = paths.remove(&path) {
                        let peer = if entry.machine_a == from {
                            entry.machine_b
                        } else if entry.machine_b == from {
                            entry.machine_a
                        } else {
                            continue;
                        };
                        Some((from, peer))
                    } else {
                        None
                    }
                };

                if let Some((severer, peer)) = notify {
                    let machines = machines.read().await;
                    if let Some(entry) = machines.get(&peer) {
                        let _ = entry.signal_tx.try_send(MachineSignal::ConnectionSevered {
                            path,
                            peer: severer.clone(),
                        });
                    }
                    // Skip duplicate for self-connections.
                    if severer != peer {
                        if let Some(entry) = machines.get(&severer) {
                            let _ = entry.signal_tx.try_send(MachineSignal::ConnectionSevered {
                                path,
                                peer: peer.clone(),
                            });
                        }
                    }
                }
            }
            PathCommand::Send { path, from, data } => {
                let peer = {
                    let paths = paths.read().await;
                    if let Some(entry) = paths.get(&path) {
                        if entry.state != PathState::Established {
                            continue; // silently drop: not established
                        }
                        if entry.machine_a == from {
                            Some(entry.machine_b.clone())
                        } else if entry.machine_b == from {
                            Some(entry.machine_a.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(peer) = peer {
                    let machines = machines.read().await;
                    if let Some(entry) = machines.get(&peer) {
                        let _ = entry
                            .signal_tx
                            .try_send(MachineSignal::IucvData { path, data });
                    }
                }
            }
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

    // ---------------------------------------------------------------
    // IUCV Path tests (Phase 10b)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn connect_ok() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, alice_handle) = collector();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        assert!(path.as_u32() > 0);

        // Both sides should receive ConnectionComplete via collector.
        wait_for(2000, || {
            alice_handle.path_event_count() >= 1 && bob_handle.path_event_count() >= 2
        })
        .await;

        // Bob gets Pending + Complete, Alice gets Complete.
        let alice_events = alice_handle.path_events();
        assert!(alice_events.iter().any(|e| matches!(e,
            crate::collector::PathEvent::Complete { path: p, peer }
            if *p == path && peer.as_str() == "BOB"
        )));

        let bob_events = bob_handle.path_events();
        assert!(bob_events.iter().any(|e| matches!(e,
            crate::collector::PathEvent::Pending { path: p, from }
            if *p == path && from.as_str() == "ALICE"
        )));

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn connect_unknown_sender() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let ghost = MachineId::new("GHOST").unwrap();

        let (h_alice, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();

        // From unknown machine.
        let err = sup.connect(&ghost, &alice).await.unwrap_err();
        assert_eq!(err.rc(), 12);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn connect_unknown_target() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let ghost = MachineId::new("GHOST").unwrap();

        let (h_alice, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();

        // To unknown machine.
        let err = sup.connect(&alice, &ghost).await.unwrap_err();
        assert_eq!(err.rc(), 12);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn connect_to_self() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();

        let (h_alice, alice_handle) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();

        let _path = sup.connect(&alice, &alice).await.unwrap();
        // Alice should get Pending (as target) and one Complete (deduped).
        wait_for(2000, || alice_handle.path_event_count() >= 2).await;
        assert_eq!(alice_handle.path_event_count(), 2);

        let paths = sup.query_paths().await;
        assert_eq!(paths.len(), 1);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn connect_after_shutdown() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        sup.shutdown().await;

        // Both machines logged off during shutdown.
        let err = sup.connect(&alice, &bob).await.unwrap_err();
        assert_eq!(err.rc(), 12); // MachineNotFound
    }

    #[tokio::test]
    async fn target_accepts_default() {
        // Default collector handler accepts all connections.
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        let paths = sup.query_paths().await;
        assert!(paths.contains(&path));

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn target_refuses() {
        struct Refuser;
        impl MachineHandler for Refuser {
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
            fn on_connection_pending(
                &mut self,
                _ctx: &MachineContext,
                _path: PathId,
                _from: &MachineId,
            ) -> bool {
                false // refuse all connections
            }
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, Refuser).await.unwrap();

        let err = sup.connect(&alice, &bob).await.unwrap_err();
        assert_eq!(err.rc(), 40); // ConnectionRefused

        // No paths should remain.
        assert!(sup.query_paths().await.is_empty());

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn connect_to_logged_off_machine() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        sup.logoff(&bob).await.unwrap();

        let err = sup.connect(&alice, &bob).await.unwrap_err();
        assert_eq!(err.rc(), 12); // MachineNotFound

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn sever_by_initiator() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, alice_handle) = collector();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        // Wait for connection to be fully established.
        wait_for(2000, || {
            alice_handle.path_event_count() >= 1 && bob_handle.path_event_count() >= 2
        })
        .await;

        sup.sever(path).await.unwrap();

        // Both sides should receive ConnectionSevered.
        wait_for(2000, || {
            alice_handle
                .path_events()
                .iter()
                .any(|e| matches!(e, crate::collector::PathEvent::Severed { .. }))
                && bob_handle
                    .path_events()
                    .iter()
                    .any(|e| matches!(e, crate::collector::PathEvent::Severed { .. }))
        })
        .await;

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn sever_by_target() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, alice_handle) = collector();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        wait_for(2000, || bob_handle.path_event_count() >= 2).await;

        // Target severs.
        sup.sever(path).await.unwrap();

        wait_for(2000, || {
            alice_handle
                .path_events()
                .iter()
                .any(|e| matches!(e, crate::collector::PathEvent::Severed { .. }))
        })
        .await;

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn sever_unknown_path() {
        let sup = Supervisor::new();

        let err = sup.sever(PathId(999)).await.unwrap_err();
        assert_eq!(err.rc(), 36); // PathNotFound

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn double_sever() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        sup.sever(path).await.unwrap();

        // Second sever should fail — entry was removed on first sever.
        let err = sup.sever(path).await.unwrap_err();
        assert_eq!(err.rc(), 36); // PathNotFound (entry removed)

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn send_data_both_directions() {
        let sup = Supervisor::new();
        struct SendOnConnect {
            data: Vec<u8>,
        }
        impl MachineHandler for SendOnConnect {
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
            fn on_connection_complete(
                &mut self,
                ctx: &MachineContext,
                path: PathId,
                _peer: &MachineId,
            ) {
                let buf = IucvBuffer::new(self.data.clone()).unwrap();
                let _ = ctx.iucv_send(path, buf);
            }
        }

        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        // Alice sends "HELLO" on connect, Bob echoes it back.
        sup.ipl(
            &alice,
            SendOnConnect {
                data: b"HELLO".to_vec(),
            },
        )
        .await
        .unwrap();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&bob, h_bob).await.unwrap();

        let _path = sup.connect(&alice, &bob).await.unwrap();

        // Bob should receive IUCV data from Alice.
        wait_for(2000, || {
            bob_handle
                .path_events()
                .iter()
                .any(|e| matches!(e, crate::collector::PathEvent::Data { .. }))
        })
        .await;

        let bob_events = bob_handle.path_events();
        let data_event = bob_events
            .iter()
            .find(|e| matches!(e, crate::collector::PathEvent::Data { .. }));
        assert!(data_event.is_some());
        if let Some(crate::collector::PathEvent::Data { data, .. }) = data_event {
            assert_eq!(data.as_bytes(), b"HELLO");
        }

        sup.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_on_pending_path() {
        // Bob blocks in on_connection_pending, keeping the path Pending.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<PathId>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();

        struct BlockingAcceptor {
            ready: std::sync::mpsc::Sender<PathId>,
            release: std::sync::mpsc::Receiver<()>,
        }
        impl MachineHandler for BlockingAcceptor {
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
            fn on_connection_pending(
                &mut self,
                _ctx: &MachineContext,
                path: PathId,
                _from: &MachineId,
            ) -> bool {
                let _ = self.ready.send(path);
                let _ = self.release.recv();
                true
            }
        }

        // Alice sends IUCV data whenever she receives an SMSG with a path id.
        struct SmsgTriggeredSend;
        impl MachineHandler for SmsgTriggeredSend {
            fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
                if let Ok(pid) = msg.text().parse::<u32>() {
                    let buf = IucvBuffer::new(b"DATA".to_vec()).unwrap();
                    let _ = ctx.iucv_send(PathId(pid), buf);
                }
            }
        }

        let sup = Arc::new(Supervisor::new());
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        sup.ipl(&alice, SmsgTriggeredSend).await.unwrap();
        sup.ipl(
            &bob,
            BlockingAcceptor {
                ready: ready_tx,
                release: release_rx,
            },
        )
        .await
        .unwrap();

        // Spawn connect in background — it blocks until Bob accepts.
        let sup2 = Arc::clone(&sup);
        let alice2 = alice.clone();
        let bob2 = bob.clone();
        let connect_task = tokio::spawn(async move { sup2.connect(&alice2, &bob2).await });

        // Wait for Bob to enter on_connection_pending (path is now Pending).
        let pending_path = ready_rx.recv().unwrap();

        // Tell Alice to send data on the pending path via SMSG trigger.
        sup.smsg(&bob, &alice, &pending_path.as_u32().to_string())
            .await
            .unwrap();

        // Release Bob's acceptor so connect completes.
        // The connect_task.await below provides synchronization — by the time
        // it returns, Alice's SMSG handler has fired and the path_cmd_loop has
        // had the opportunity to process (and silently drop) the send.
        drop(release_tx);
        let _path = connect_task.await.unwrap().unwrap();

        // Bob should have received Pending + Complete events, but NO Data event
        // (the send while pending should have been silently dropped).
        // Note: Bob's handler is BlockingAcceptor, not a collector. We can't
        // inspect Bob's events. But we can verify via the path_cmd_loop behavior.
        // The key assertion is that this test doesn't hang or panic.

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn send_on_severed_path() {
        struct SendOnSmsg;
        impl MachineHandler for SendOnSmsg {
            fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
                if let Ok(pid) = msg.text().parse::<u32>() {
                    let buf = IucvBuffer::new(b"AFTER_SEVER".to_vec()).unwrap();
                    let _ = ctx.iucv_send(PathId(pid), buf);
                }
            }
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        sup.ipl(&alice, SendOnSmsg).await.unwrap();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        wait_for(2000, || bob_handle.path_event_count() >= 2).await;

        // Sever the path.
        sup.sever(path).await.unwrap();
        wait_for(2000, || {
            bob_handle
                .path_events()
                .iter()
                .any(|e| matches!(e, crate::collector::PathEvent::Severed { .. }))
        })
        .await;

        let count_before = bob_handle
            .path_events()
            .iter()
            .filter(|e| matches!(e, crate::collector::PathEvent::Data { .. }))
            .count();

        // Tell Alice to send data on the severed path.
        sup.smsg(&bob, &alice, &path.as_u32().to_string())
            .await
            .unwrap();
        // Send a "fence" SMSG to Bob (not a path ID, so no side effects).
        // When Bob receives it, we know the runtime has processed all prior
        // messages including Alice's SMSG and the resulting path_cmd_loop work.
        let msg_count_before = bob_handle.count();
        sup.smsg(&alice, &bob, "FENCE").await.unwrap();
        wait_for(2000, || bob_handle.count() > msg_count_before).await;

        // Bob should NOT have received any new Data events.
        let count_after = bob_handle
            .path_events()
            .iter()
            .filter(|e| matches!(e, crate::collector::PathEvent::Data { .. }))
            .count();
        assert_eq!(count_before, count_after);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn send_large_buffer() {
        struct SendOnConnect {
            data: Vec<u8>,
        }
        impl MachineHandler for SendOnConnect {
            fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
            fn on_connection_complete(
                &mut self,
                ctx: &MachineContext,
                path: PathId,
                _peer: &MachineId,
            ) {
                let buf = IucvBuffer::new(self.data.clone()).unwrap();
                let _ = ctx.iucv_send(path, buf);
            }
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        // Send near-max IUCV buffer (65535 bytes).
        let big_data = vec![0xABu8; 65535];
        sup.ipl(
            &alice,
            SendOnConnect {
                data: big_data.clone(),
            },
        )
        .await
        .unwrap();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&bob, h_bob).await.unwrap();

        let _path = sup.connect(&alice, &bob).await.unwrap();

        wait_for(2000, || {
            bob_handle
                .path_events()
                .iter()
                .any(|e| matches!(e, crate::collector::PathEvent::Data { .. }))
        })
        .await;

        let bob_events = bob_handle.path_events();
        let data_event = bob_events
            .iter()
            .find(|e| matches!(e, crate::collector::PathEvent::Data { .. }));
        if let Some(crate::collector::PathEvent::Data { data, .. }) = data_event {
            assert_eq!(data.len(), 65535);
            assert_eq!(data.as_bytes()[0], 0xAB);
        } else {
            panic!("Expected Data event");
        }

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn logoff_severs_paths() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        wait_for(2000, || bob_handle.path_event_count() >= 2).await;

        // Alice logs off — path should be severed.
        sup.logoff(&alice).await.unwrap();

        // Bob should receive ConnectionSevered.
        wait_for(2000, || {
            bob_handle
                .path_events()
                .iter()
                .any(|e| matches!(e, crate::collector::PathEvent::Severed { .. }))
        })
        .await;

        // Path should no longer be active.
        let active = sup.query_paths().await;
        assert!(!active.contains(&path));

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn peer_receives_severed_on_logoff() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, alice_handle) = collector();
        let (h_bob, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        wait_for(2000, || alice_handle.path_event_count() >= 1).await;

        // Bob logs off — Alice (the initiator/peer) should be notified.
        sup.logoff(&bob).await.unwrap();

        wait_for(2000, || {
            alice_handle.path_events().iter().any(|e| {
                matches!(e,
                    crate::collector::PathEvent::Severed { path: p, peer }
                    if *p == path && peer.as_str() == "BOB"
                )
            })
        })
        .await;

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_severs_all_paths() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();
        let carol = MachineId::new("CAROL").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, _) = collector();
        let (h_carol, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();
        sup.ipl(&carol, h_carol).await.unwrap();

        let _p1 = sup.connect(&alice, &bob).await.unwrap();
        let _p2 = sup.connect(&bob, &carol).await.unwrap();

        assert_eq!(sup.query_paths().await.len(), 2);

        sup.shutdown().await;

        assert!(sup.query_paths().await.is_empty());
    }

    #[tokio::test]
    async fn sever_path_from_handler() {
        struct SeverOnSmsg;
        impl MachineHandler for SeverOnSmsg {
            fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
                if let Ok(pid) = msg.text().parse::<u32>() {
                    let _ = ctx.sever_path(PathId(pid));
                }
            }
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        sup.ipl(&alice, SeverOnSmsg).await.unwrap();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        wait_for(2000, || bob_handle.path_event_count() >= 2).await;

        // Tell Alice to sever the path via SMSG.
        sup.smsg(&bob, &alice, &path.as_u32().to_string())
            .await
            .unwrap();

        // Bob should receive ConnectionSevered from the path_cmd_loop.
        wait_for(2000, || {
            bob_handle
                .path_events()
                .iter()
                .any(|e| matches!(e, crate::collector::PathEvent::Severed { .. }))
        })
        .await;

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn iucv_send_from_handler() {
        struct SendOnSmsg;
        impl MachineHandler for SendOnSmsg {
            fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
                if let Ok(pid) = msg.text().parse::<u32>() {
                    let buf = IucvBuffer::new(b"FROM_HANDLER".to_vec()).unwrap();
                    let _ = ctx.iucv_send(PathId(pid), buf);
                }
            }
        }

        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        sup.ipl(&alice, SendOnSmsg).await.unwrap();
        let (h_bob, bob_handle) = collector();
        sup.ipl(&bob, h_bob).await.unwrap();

        let path = sup.connect(&alice, &bob).await.unwrap();
        wait_for(2000, || bob_handle.path_event_count() >= 2).await;

        // Tell Alice to send IUCV data via SMSG trigger.
        sup.smsg(&bob, &alice, &path.as_u32().to_string())
            .await
            .unwrap();

        // Bob should receive the IUCV data.
        wait_for(2000, || {
            bob_handle
                .path_events()
                .iter()
                .any(|e| matches!(e, crate::collector::PathEvent::Data { .. }))
        })
        .await;

        let events = bob_handle.path_events();
        let data_event = events
            .iter()
            .find(|e| matches!(e, crate::collector::PathEvent::Data { .. }));
        if let Some(crate::collector::PathEvent::Data { data, .. }) = data_event {
            assert_eq!(data.as_bytes(), b"FROM_HANDLER");
        } else {
            panic!("Expected Data event");
        }

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn query_paths_filters_severed() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();
        let carol = MachineId::new("CAROL").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, _) = collector();
        let (h_carol, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();
        sup.ipl(&carol, h_carol).await.unwrap();

        let p1 = sup.connect(&alice, &bob).await.unwrap();
        let p2 = sup.connect(&alice, &carol).await.unwrap();
        assert_eq!(sup.query_paths().await.len(), 2);

        // Sever one path.
        sup.sever(p1).await.unwrap();

        let paths = sup.query_paths().await;
        assert_eq!(paths.len(), 1);
        assert!(paths.contains(&p2));
        assert!(!paths.contains(&p1));

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn multiple_paths_between_same_machines() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, _) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();

        let p1 = sup.connect(&alice, &bob).await.unwrap();
        let p2 = sup.connect(&alice, &bob).await.unwrap();
        assert_ne!(p1, p2);

        let paths = sup.query_paths().await;
        assert_eq!(paths.len(), 2);

        sup.shutdown().await;
    }

    #[tokio::test]
    async fn logoff_severs_multiple_paths() {
        let sup = Supervisor::new();
        let alice = MachineId::new("ALICE").unwrap();
        let bob = MachineId::new("BOB").unwrap();
        let carol = MachineId::new("CAROL").unwrap();

        let (h_alice, _) = collector();
        let (h_bob, bob_handle) = collector();
        let (h_carol, carol_handle) = collector();
        sup.ipl(&alice, h_alice).await.unwrap();
        sup.ipl(&bob, h_bob).await.unwrap();
        sup.ipl(&carol, h_carol).await.unwrap();

        let _p1 = sup.connect(&alice, &bob).await.unwrap();
        let _p2 = sup.connect(&alice, &carol).await.unwrap();
        wait_for(2000, || {
            bob_handle.path_event_count() >= 2 && carol_handle.path_event_count() >= 2
        })
        .await;

        assert_eq!(sup.query_paths().await.len(), 2);

        // Alice logs off — both paths should be severed.
        sup.logoff(&alice).await.unwrap();

        wait_for(2000, || {
            bob_handle
                .path_events()
                .iter()
                .any(|e| matches!(e, crate::collector::PathEvent::Severed { .. }))
                && carol_handle
                    .path_events()
                    .iter()
                    .any(|e| matches!(e, crate::collector::PathEvent::Severed { .. }))
        })
        .await;

        // No active paths remaining (all severed).
        let active = sup.query_paths().await;
        assert!(
            active.is_empty(),
            "Expected no active paths after logoff, got {}",
            active.len()
        );

        sup.shutdown().await;
    }
}
