use std::sync::mpsc;

use cms_core::minidisk::AccessMode;
use cms_core::{CmsFileSystem, FileSpec};
use cms_machine::handler::CmsMachineHandler;
use cms_machine::CmsExtCommandHandler;
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::machine_id::MachineId;
use vm_iucv::message::SmsgMessage;
use vm_iucv::supervisor::Supervisor;

struct ConHandler;
impl MachineHandler for ConHandler {
    fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
}

/// Poll the output channel until `predicate` returns true or timeout expires.
async fn poll_output_until(
    output_rx: &mpsc::Receiver<String>,
    predicate: impl Fn(&[String]) -> bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        while let Ok(line) = output_rx.try_recv() {
            lines.push(line);
        }
        if predicate(&lines) {
            return lines;
        }
        if tokio::time::Instant::now() >= deadline {
            return lines;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Drain all pending output from the channel, waiting for the BATCH_DONE
/// sentinel if one is expected. Falls back to a short timeout for operations
/// that don't send a sentinel (e.g., inbound SMSG from non-$CON).
async fn drain_output(output_rx: &mpsc::Receiver<String>) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(200);
    loop {
        while let Ok(line) = output_rx.try_recv() {
            if line == CmsMachineHandler::BATCH_DONE {
                return;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn ipl_prints_banner() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut fs = CmsFileSystem::new();
    fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
        .unwrap();

    let (_cmd_tx, cmd_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();

    let handler = CmsMachineHandler::without_rexx(fs, cmd_rx, output_tx);

    let sup = Supervisor::new();
    let con_id = MachineId::new("$CON").unwrap();
    let user_id = MachineId::new("USER").unwrap();

    sup.ipl(&con_id, ConHandler).await.unwrap();
    sup.ipl(&user_id, handler).await.unwrap();

    let lines =
        poll_output_until(&output_rx, |l| l.iter().any(|s| s.contains("CMS Machine"))).await;

    assert!(
        lines.iter().any(|l| l.contains("CMS Machine Environment")),
        "Expected banner, got: {:?}",
        lines,
    );

    sup.shutdown().await;
}

#[tokio::test]
async fn console_command_via_smsg_wake() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut fs = CmsFileSystem::new();
    fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
        .unwrap();

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();

    let handler = CmsMachineHandler::without_rexx(fs, cmd_rx, output_tx);

    let sup = Supervisor::new();
    let con_id = MachineId::new("$CON").unwrap();
    let user_id = MachineId::new("USER").unwrap();

    sup.ipl(&con_id, ConHandler).await.unwrap();
    sup.ipl(&user_id, handler).await.unwrap();

    // Wait for IPL to complete, then drain
    poll_output_until(&output_rx, |l| l.iter().any(|s| s.contains("CMS Machine"))).await;
    drain_output(&output_rx).await;

    // Send a GLOBALV SET command
    cmd_tx.send("GLOBALV SET COLOR blue".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();
    drain_output(&output_rx).await;

    // Now send GET
    cmd_tx.send("GLOBALV GET COLOR".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();

    let lines = poll_output_until(&output_rx, |l| l.contains(&"blue".to_string())).await;

    assert!(
        lines.contains(&"blue".to_string()),
        "Expected 'blue' in output, got: {:?}",
        lines,
    );

    sup.shutdown().await;
}

#[tokio::test]
async fn inbound_smsg_sets_globalv() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut fs = CmsFileSystem::new();
    fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
        .unwrap();

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();

    let handler = CmsMachineHandler::without_rexx(fs, cmd_rx, output_tx);

    let sup = Supervisor::new();
    let con_id = MachineId::new("$CON").unwrap();
    let user_id = MachineId::new("USER").unwrap();
    let oper_id = MachineId::new("OPER").unwrap();

    sup.ipl(&con_id, ConHandler).await.unwrap();
    sup.ipl(&user_id, handler).await.unwrap();
    sup.ipl(&oper_id, ConHandler).await.unwrap();

    // Wait for IPL, then drain
    poll_output_until(&output_rx, |l| l.iter().any(|s| s.contains("CMS Machine"))).await;
    drain_output(&output_rx).await;

    // Send SMSG from OPER to USER
    sup.smsg(&oper_id, &user_id, "Hello from OPER")
        .await
        .unwrap();

    let lines = poll_output_until(&output_rx, |l| {
        l.iter().any(|s| s.contains("MSG FROM OPER"))
    })
    .await;

    assert!(
        lines.iter().any(|l| l.contains("MSG FROM OPER")),
        "Expected MSG FROM OPER, got: {:?}",
        lines,
    );

    // Verify GLOBALV was set — send GET via console
    cmd_tx.send("GLOBALV GET LMSGSRC".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();

    let lines2 = poll_output_until(&output_rx, |l| l.contains(&"OPER".to_string())).await;

    assert!(
        lines2.contains(&"OPER".to_string()),
        "Expected LMSGSRC=OPER, got: {:?}",
        lines2,
    );

    sup.shutdown().await;
}

#[cfg(feature = "rexx")]
#[tokio::test]
async fn rexx_exec_runs_via_console_with_swap() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut fs = CmsFileSystem::new();
    fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
        .unwrap();

    // Write a simple EXEC to disk
    let spec = FileSpec::parse("HELLO EXEC A").unwrap();
    fs.write_file(&spec, "/* REXX */\n'GLOBALV SET GREETING Hi there'\n")
        .unwrap();

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();

    // Use swap handler for persistent state
    let exec_handler = Box::new(cms_machine::rexx_exec::CmsRexxExecHandlerWithSwap::new());
    let ext_handler = Box::new(CmsExtCommandHandler::new("USER"));
    let handler = CmsMachineHandler::new(fs, exec_handler, ext_handler, cmd_rx, output_tx);

    let sup = Supervisor::new();
    let con_id = MachineId::new("$CON").unwrap();
    let user_id = MachineId::new("USER").unwrap();

    sup.ipl(&con_id, ConHandler).await.unwrap();
    sup.ipl(&user_id, handler).await.unwrap();

    // Wait for IPL, then drain
    poll_output_until(&output_rx, |l| l.iter().any(|s| s.contains("CMS Machine"))).await;
    drain_output(&output_rx).await;

    // Run the EXEC (will be found as unknown command → fallback)
    cmd_tx.send("HELLO".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();
    drain_output(&output_rx).await;

    // Verify GLOBALV WAS set (swap handler persists state)
    cmd_tx.send("GLOBALV GET GREETING".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();

    let lines = poll_output_until(&output_rx, |l| l.contains(&"Hi there".to_string())).await;

    assert!(
        lines.contains(&"Hi there".to_string()),
        "Expected 'Hi there' in output (swap handler persists state), got: {:?}",
        lines,
    );

    sup.shutdown().await;
}

#[cfg(feature = "rexx")]
#[tokio::test]
async fn spool_command_via_console() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut fs = CmsFileSystem::new();
    fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
        .unwrap();

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();

    let exec_handler = Box::new(cms_machine::rexx_exec::CmsRexxExecHandlerWithSwap::new());
    let ext_handler = Box::new(CmsExtCommandHandler::new("USER"));
    let handler = CmsMachineHandler::new(fs, exec_handler, ext_handler, cmd_rx, output_tx);

    let sup = Supervisor::new();
    let con_id = MachineId::new("$CON").unwrap();
    let user_id = MachineId::new("USER").unwrap();

    sup.ipl(&con_id, ConHandler).await.unwrap();
    sup.ipl(&user_id, handler).await.unwrap();

    poll_output_until(&output_rx, |l| l.iter().any(|s| s.contains("CMS Machine"))).await;
    drain_output(&output_rx).await;

    // SP PRT CLASS B
    cmd_tx.send("SP PRT CLASS B".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();

    let lines = poll_output_until(&output_rx, |l| l.iter().any(|s| s.contains("PRINTER"))).await;

    assert!(
        lines.iter().any(|l| l.contains("PRINTER")),
        "Expected PRINTER in spool output, got: {:?}",
        lines,
    );

    sup.shutdown().await;
}

#[cfg(feature = "rexx")]
#[tokio::test]
async fn query_reader_via_console() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut fs = CmsFileSystem::new();
    fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
        .unwrap();

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();

    let exec_handler = Box::new(cms_machine::rexx_exec::CmsRexxExecHandlerWithSwap::new());
    let ext_handler = Box::new(CmsExtCommandHandler::new("USER"));
    let handler = CmsMachineHandler::new(fs, exec_handler, ext_handler, cmd_rx, output_tx);

    let sup = Supervisor::new();
    let con_id = MachineId::new("$CON").unwrap();
    let user_id = MachineId::new("USER").unwrap();

    sup.ipl(&con_id, ConHandler).await.unwrap();
    sup.ipl(&user_id, handler).await.unwrap();

    poll_output_until(&output_rx, |l| l.iter().any(|s| s.contains("CMS Machine"))).await;
    drain_output(&output_rx).await;

    // Q R
    cmd_tx.send("Q R".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();

    let lines = poll_output_until(&output_rx, |l| l.iter().any(|s| s.contains("No files"))).await;

    assert!(
        lines.iter().any(|l| l.contains("No files")),
        "Expected 'No files' in query output, got: {:?}",
        lines,
    );

    sup.shutdown().await;
}

#[cfg(feature = "rexx")]
#[tokio::test]
async fn pipe_command_via_console() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut fs = CmsFileSystem::new();
    fs.access_disk('A', dir.path().join("a"), AccessMode::ReadWrite)
        .unwrap();

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();

    let exec_handler = Box::new(cms_machine::rexx_exec::CmsRexxExecHandlerWithSwap::new());
    let ext_handler = Box::new(CmsExtCommandHandler::new("USER"));
    let handler = CmsMachineHandler::new(fs, exec_handler, ext_handler, cmd_rx, output_tx);

    let sup = Supervisor::new();
    let con_id = MachineId::new("$CON").unwrap();
    let user_id = MachineId::new("USER").unwrap();

    sup.ipl(&con_id, ConHandler).await.unwrap();
    sup.ipl(&user_id, handler).await.unwrap();

    poll_output_until(&output_rx, |l| l.iter().any(|s| s.contains("CMS Machine"))).await;
    drain_output(&output_rx).await;

    // PIPE literal hello | console
    cmd_tx
        .send("PIPE literal hello | console".to_string())
        .unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();

    let lines = poll_output_until(&output_rx, |l| l.contains(&"hello".to_string())).await;

    assert!(
        lines.contains(&"hello".to_string()),
        "Expected 'hello' in pipeline output, got: {:?}",
        lines,
    );

    sup.shutdown().await;
}
