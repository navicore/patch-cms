use cms_core::{CmsFileSystem, CommandProcessor, ExecHandler, NoExecHandler, SmsgSender};
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::message::SmsgMessage;

use std::sync::mpsc;

/// SMSG sender that posts messages to a channel for later delivery by the handler.
///
/// The handler drains this channel after each `execute()` call and forwards
/// the messages via `ctx.try_send_smsg()`.
pub struct ChannelSmsgSender {
    tx: mpsc::Sender<(String, String)>,
}

impl ChannelSmsgSender {
    pub fn new(tx: mpsc::Sender<(String, String)>) -> Self {
        ChannelSmsgSender { tx }
    }
}

impl SmsgSender for ChannelSmsgSender {
    fn send_smsg(&self, target: &str, text: &str) -> (i32, String) {
        match self.tx.send((target.to_string(), text.to_string())) {
            Ok(()) => (0, String::new()),
            Err(_) => (28, "SMSG channel closed".to_string()),
        }
    }
}

/// Handler for an interactive CMS machine.
///
/// Processes commands received from the console (via `$CON` SMSG wake pattern)
/// and handles inbound SMSGs from other machines.
pub struct CmsMachineHandler {
    processor: CommandProcessor,
    cmd_rx: Option<mpsc::Receiver<String>>,
    output_tx: mpsc::Sender<String>,
    smsg_rx: mpsc::Receiver<(String, String)>,
}

impl CmsMachineHandler {
    /// Create a new CMS machine handler.
    ///
    /// - `filesystem`: The CMS filesystem with accessed minidisks.
    /// - `exec_handler`: REXX EXEC handler.
    /// - `cmd_rx`: Receives command strings from the console thread.
    /// - `output_tx`: Sends output lines back to the console thread.
    pub fn new(
        filesystem: CmsFileSystem,
        exec_handler: Box<dyn ExecHandler>,
        cmd_rx: mpsc::Receiver<String>,
        output_tx: mpsc::Sender<String>,
    ) -> Self {
        let (smsg_tx, smsg_rx) = mpsc::channel();
        let sender = ChannelSmsgSender::new(smsg_tx);
        let processor =
            CommandProcessor::with_smsg_sender(filesystem, exec_handler, Box::new(sender));
        CmsMachineHandler {
            processor,
            cmd_rx: Some(cmd_rx),
            output_tx,
            smsg_rx,
        }
    }

    /// Create a handler with no EXEC support (for testing without REXX).
    pub fn without_rexx(
        filesystem: CmsFileSystem,
        cmd_rx: mpsc::Receiver<String>,
        output_tx: mpsc::Sender<String>,
    ) -> Self {
        Self::new(filesystem, Box::new(NoExecHandler), cmd_rx, output_tx)
    }

    fn drain_commands(&mut self, ctx: &MachineContext) {
        let rx = match self.cmd_rx.as_ref() {
            Some(rx) => rx,
            None => return,
        };

        while let Ok(cmd_line) = rx.try_recv() {
            let result = self.processor.execute(&cmd_line);
            for msg in &result.messages {
                let _ = self.output_tx.send(msg.clone());
            }
            if result.rc != 0 {
                let _ = self.output_tx.send(format!("RC={}", result.rc));
            }
            // Drain outbound SMSGs produced by this command
            self.drain_outbound_smsgs(ctx);
        }
    }

    fn drain_outbound_smsgs(&self, ctx: &MachineContext) {
        while let Ok((target, text)) = self.smsg_rx.try_recv() {
            if let Ok(target_id) = vm_iucv::machine_id::MachineId::new(&target) {
                let _ = ctx.try_send_smsg(&target_id, &text);
            } else {
                let _ = self
                    .output_tx
                    .send(format!("DMSMSG057E Invalid userid: {}", target));
            }
        }
    }
}

impl MachineHandler for CmsMachineHandler {
    fn on_ipl(&mut self, _ctx: &MachineContext) {
        let _ = self
            .output_tx
            .send("z/VM CMS Machine Environment".to_string());
        let _ = self.output_tx.send(String::new());

        // Run PROFILE EXEC if found
        if let Some(result) = self.processor.run_profile() {
            for msg in &result.messages {
                let _ = self.output_tx.send(msg.clone());
            }
        }
    }

    fn on_smsg(&mut self, ctx: &MachineContext, msg: SmsgMessage) {
        let sender = msg.from().as_str();

        if sender == "$CON" {
            // Console wake: drain pending commands
            self.drain_commands(ctx);
        } else {
            // Inbound SMSG from another machine
            let _ = self
                .output_tx
                .send(format!("* MSG FROM {} -- {}", sender, msg.text()));

            // Store in GLOBALV LASTING
            let gv = self.processor.globalv_mut();
            let prev_group = gv.current_group().to_string();
            gv.select("LASTING");
            gv.set("LMSG", &format!("MSG FROM {} -- {}", sender, msg.text()));
            gv.set("LMSGSRC", sender);
            gv.set("LMSGTXT", msg.text());
            if prev_group != "LASTING" {
                gv.select(&prev_group);
            }
        }
    }

    fn on_logoff(&mut self, _ctx: &MachineContext) {
        let _ = self.output_tx.send("LOGOFF AT".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cms_core::minidisk::AccessMode;
    use tempfile::TempDir;

    fn setup_handler() -> (
        TempDir,
        CmsMachineHandler,
        mpsc::Sender<String>,
        mpsc::Receiver<String>,
    ) {
        let dir = TempDir::new().unwrap();
        let mut fs = CmsFileSystem::new();
        fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
            .unwrap();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (output_tx, output_rx) = mpsc::channel();
        let handler = CmsMachineHandler::without_rexx(fs, cmd_rx, output_tx);
        (dir, handler, cmd_tx, output_rx)
    }

    #[test]
    fn channel_smsg_sender_delivers() {
        let (tx, rx) = mpsc::channel();
        let sender = ChannelSmsgSender::new(tx);
        let (rc, _) = sender.send_smsg("OPER", "Hello");
        assert_eq!(rc, 0);
        let (target, text) = rx.recv().unwrap();
        assert_eq!(target, "OPER");
        assert_eq!(text, "Hello");
    }

    #[test]
    fn handler_processes_globalv() {
        let (_dir, mut handler, _cmd_tx, _output_rx) = setup_handler();

        let result = handler.processor.execute("GLOBALV SET COLOR blue");
        assert_eq!(result.rc, 0);
        let result = handler.processor.execute("GLOBALV GET COLOR");
        assert_eq!(result.rc, 0);
        assert_eq!(result.messages, vec!["blue"]);
    }

    #[test]
    fn handler_processes_smsg_command() {
        let (_dir, mut handler, _cmd_tx, _output_rx) = setup_handler();

        // ChannelSmsgSender posts to the internal channel — RC=0
        let result = handler.processor.execute("SMSG OPER test");
        assert_eq!(result.rc, 0);
    }
}
