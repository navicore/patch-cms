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

    /// Send an SMSG to another machine via the supervisor router.
    ///
    /// The message is enqueued in the router's input channel. Delivery is
    /// best-effort: if the target machine logs off between enqueue and
    /// dispatch, the message is silently dropped by the router.
    pub async fn send_smsg(&self, to: &MachineId, text: &str) -> Result<()> {
        let msg = SmsgMessage {
            from: self.machine_id.clone(),
            to: to.clone(),
            text: text.to_string(),
        };
        self.outbox
            .send(msg)
            .await
            .map_err(|_| IucvError::SupervisorDown)
    }

    /// Try to send an SMSG without awaiting (for use in sync handler callbacks).
    ///
    /// The message is enqueued in the router's input channel. Delivery is
    /// best-effort: if the target machine logs off between enqueue and
    /// dispatch, the message is silently dropped by the router.
    pub fn try_send_smsg(&self, to: &MachineId, text: &str) -> Result<()> {
        let msg = SmsgMessage {
            from: self.machine_id.clone(),
            to: to.clone(),
            text: text.to_string(),
        };
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
pub trait MachineHandler: Send + 'static {
    /// Called once after the machine is IPL'd (booted).
    fn on_ipl(&mut self, _ctx: &MachineContext) {}

    /// Called for each incoming SMSG. Every handler must implement this.
    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage);

    /// Called once when the machine is being logged off.
    fn on_logoff(&mut self, _ctx: &MachineContext) {}
}
