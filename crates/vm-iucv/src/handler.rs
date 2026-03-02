use crate::error::{IucvError, Result};
use crate::machine_id::MachineId;
use crate::message::SmsgMessage;
use crate::path::{IucvBuffer, PathId};
use tokio::sync::mpsc;

/// Commands for path lifecycle, sent from MachineContext to the supervisor.
pub(crate) enum PathCommand {
    Sever {
        path: PathId,
        from: MachineId,
    },
    Send {
        path: PathId,
        from: MachineId,
        data: IucvBuffer,
    },
}

/// Runtime context passed to a machine handler during lifecycle callbacks.
pub struct MachineContext {
    machine_id: MachineId,
    outbox: mpsc::Sender<SmsgMessage>,
    path_cmd_tx: mpsc::Sender<PathCommand>,
}

impl MachineContext {
    pub(crate) fn new(
        machine_id: MachineId,
        outbox: mpsc::Sender<SmsgMessage>,
        path_cmd_tx: mpsc::Sender<PathCommand>,
    ) -> Self {
        MachineContext {
            machine_id,
            outbox,
            path_cmd_tx,
        }
    }

    /// The identity of this machine.
    pub fn machine_id(&self) -> &MachineId {
        &self.machine_id
    }

    /// Try to send an SMSG without awaiting (for use in sync handler callbacks).
    ///
    /// The message is enqueued in the router's input channel. Delivery is
    /// best-effort: if the target machine logs off between enqueue and
    /// dispatch, the message is silently dropped by the router.
    pub fn try_send_smsg(&self, to: &MachineId, text: &str) -> Result<()> {
        let msg = SmsgMessage::new(self.machine_id.clone(), to.clone(), text)?;
        self.outbox.try_send(msg).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => IucvError::ChannelBusy("ROUTER".to_string()),
            mpsc::error::TrySendError::Closed(_) => IucvError::SupervisorDown,
        })
    }

    /// Sever an established path from this machine's side.
    pub fn sever_path(&self, path: PathId) -> Result<()> {
        self.path_cmd_tx
            .try_send(PathCommand::Sever {
                path,
                from: self.machine_id.clone(),
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => {
                    IucvError::ChannelBusy("PATH_CMD".to_string())
                }
                mpsc::error::TrySendError::Closed(_) => IucvError::SupervisorDown,
            })
    }

    /// Send data on an established path.
    pub fn iucv_send(&self, path: PathId, data: IucvBuffer) -> Result<()> {
        self.path_cmd_tx
            .try_send(PathCommand::Send {
                path,
                from: self.machine_id.clone(),
                data,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => {
                    IucvError::ChannelBusy("PATH_CMD".to_string())
                }
                mpsc::error::TrySendError::Closed(_) => IucvError::SupervisorDown,
            })
    }
}

/// Trait implemented by each machine's message handler.
///
/// Analogous to the `Stage` trait in cms-pipelines — each machine is an
/// actor that reacts to lifecycle events and incoming messages.
///
/// All callbacks are synchronous and run on the machine's Tokio task.
/// A blocking callback prevents the machine from processing further signals
/// (including `Logoff`) until it returns. Implementations should avoid
/// long-running or blocking operations; use
/// [`MachineContext::try_send_smsg`] to send messages from within callbacks.
pub trait MachineHandler: Send + 'static {
    /// Called once after the machine is IPL'd (booted).
    ///
    /// Runs before the machine begins processing signals. A blocking
    /// `on_ipl` delays signal processing (including `Logoff`) until it
    /// returns.
    fn on_ipl(&mut self, _ctx: &MachineContext) {}

    /// Called for each incoming SMSG. Every handler must implement this.
    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage);

    /// Called when another machine requests a connection.
    /// Return true to accept, false to refuse.
    fn on_connection_pending(
        &mut self,
        _ctx: &MachineContext,
        _path: PathId,
        _from: &MachineId,
    ) -> bool {
        true // default: accept all connections
    }

    /// Called when a connection is fully established (both sides accepted).
    ///
    /// Delivery is best-effort via `try_send`: if the machine's signal channel
    /// is full, this callback may be silently skipped. Handlers should not
    /// assume that every `connect()` that succeeds will produce a matching
    /// `on_connection_complete` callback.
    fn on_connection_complete(&mut self, _ctx: &MachineContext, _path: PathId, _peer: &MachineId) {}

    /// Called when a path is severed (by either side or by logoff).
    ///
    /// Delivery is best-effort via `try_send`: if the machine's signal channel
    /// is full, this callback may be silently skipped. Handlers requiring
    /// guaranteed cleanup should use RAII guards rather than relying on this
    /// callback.
    ///
    /// **Not called on the machine that is logging off.** When a machine logs
    /// off, only its peers receive `on_connection_severed`; the logging-off
    /// machine receives `on_logoff` instead. Handlers that perform path-level
    /// cleanup should enumerate known paths in `on_logoff`.
    fn on_connection_severed(&mut self, _ctx: &MachineContext, _path: PathId, _peer: &MachineId) {}

    /// Called when data arrives on an established path.
    fn on_iucv_data(&mut self, _ctx: &MachineContext, _path: PathId, _data: IucvBuffer) {}

    /// Called once when the machine is being logged off.
    ///
    /// `Smsg` signals that were in-flight in the router when `Logoff` was
    /// enqueued may or may not be delivered before `on_logoff` is called,
    /// depending on channel ordering.
    ///
    /// **Not called if `on_ipl` or `on_smsg` panics** — the task unwinds
    /// past this hook. Use RAII guards for guaranteed cleanup.
    fn on_logoff(&mut self, _ctx: &MachineContext) {}
}
