use crate::error::{IucvError, Result};
use crate::machine_id::MachineId;
use crate::message::SmsgMessage;
use tokio::sync::mpsc;

/// Runtime context passed to a machine handler during lifecycle callbacks.
pub struct MachineContext {
    machine_id: MachineId,
    outbox: mpsc::Sender<SmsgMessage>,
}

impl MachineContext {
    pub(crate) fn new(machine_id: MachineId, outbox: mpsc::Sender<SmsgMessage>) -> Self {
        MachineContext { machine_id, outbox }
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
}

/// Trait implemented by each machine's message handler.
///
/// Analogous to the `Stage` trait in cms-pipelines — each machine is an
/// actor that reacts to lifecycle events and incoming messages.
///
/// All callbacks are synchronous and run on the machine's Tokio task.
/// A blocking callback prevents the machine from processing further signals
/// (including `Logoff`) until it returns. Implementations should avoid
/// long-running or blocking operations; use `try_send_smsg` (not the async
/// `send_smsg`) to send messages from within callbacks.
pub trait MachineHandler: Send + 'static {
    /// Called once after the machine is IPL'd (booted).
    ///
    /// Runs before the machine begins processing signals. A blocking
    /// `on_ipl` delays signal processing (including `Logoff`) until it
    /// returns.
    fn on_ipl(&mut self, _ctx: &MachineContext) {}

    /// Called for each incoming SMSG. Every handler must implement this.
    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage);

    /// Called once when the machine is being logged off.
    ///
    /// `Smsg` signals that were in-flight in the router when `Logoff` was
    /// enqueued may or may not be delivered before `on_logoff` is called,
    /// depending on channel ordering.
    fn on_logoff(&mut self, _ctx: &MachineContext) {}
}
