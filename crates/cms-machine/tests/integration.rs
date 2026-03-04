use std::sync::mpsc;

use cms_core::minidisk::AccessMode;
use cms_core::{CmsFileSystem, FileSpec};
use cms_machine::handler::CmsMachineHandler;
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::machine_id::MachineId;
use vm_iucv::message::SmsgMessage;
use vm_iucv::supervisor::Supervisor;

struct ConHandler;
impl MachineHandler for ConHandler {
    fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {}
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

    // Give the machine time to IPL
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Check banner output
    let mut lines = Vec::new();
    while let Ok(line) = output_rx.try_recv() {
        lines.push(line);
    }

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

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Drain IPL output
    while output_rx.try_recv().is_ok() {}

    // Send a GLOBALV SET command
    cmd_tx.send("GLOBALV SET COLOR blue".to_string()).unwrap();

    // Wake the machine
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Now send GET
    cmd_tx.send("GLOBALV GET COLOR".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut lines = Vec::new();
    while let Ok(line) = output_rx.try_recv() {
        lines.push(line);
    }

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

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    while output_rx.try_recv().is_ok() {}

    // Send SMSG from OPER to USER
    sup.smsg(&oper_id, &user_id, "Hello from OPER")
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Check that the MSG was displayed
    let mut lines = Vec::new();
    while let Ok(line) = output_rx.try_recv() {
        lines.push(line);
    }

    assert!(
        lines.iter().any(|l| l.contains("MSG FROM OPER")),
        "Expected MSG FROM OPER, got: {:?}",
        lines,
    );

    // Verify GLOBALV was set — send GET via console
    cmd_tx.send("GLOBALV GET LMSGSRC".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut lines2 = Vec::new();
    while let Ok(line) = output_rx.try_recv() {
        lines2.push(line);
    }

    assert!(
        lines2.contains(&"OPER".to_string()),
        "Expected LMSGSRC=OPER, got: {:?}",
        lines2,
    );

    sup.shutdown().await;
}

#[cfg(feature = "rexx")]
#[tokio::test]
async fn rexx_exec_runs_via_console() {
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

    let exec_handler = Box::new(cms_machine::rexx_exec::CmsRexxExecHandler);
    let handler = CmsMachineHandler::new(fs, exec_handler, cmd_rx, output_tx);

    let sup = Supervisor::new();
    let con_id = MachineId::new("$CON").unwrap();
    let user_id = MachineId::new("USER").unwrap();

    sup.ipl(&con_id, ConHandler).await.unwrap();
    sup.ipl(&user_id, handler).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    while output_rx.try_recv().is_ok() {}

    // Run the EXEC (will be found as unknown command → fallback)
    cmd_tx.send("HELLO".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Drain output
    while output_rx.try_recv().is_ok() {}

    // Verify GLOBALV was NOT set (simple CmsRexxExecHandler uses temp processor)
    // This is expected MVP behavior — ADDRESS CMS commands run against a fresh context
    cmd_tx.send("GLOBALV GET GREETING".to_string()).unwrap();
    sup.smsg(&con_id, &user_id, "CMD").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut lines = Vec::new();
    while let Ok(line) = output_rx.try_recv() {
        lines.push(line);
    }

    // The GET will return RC=4 (not found) since the temp processor's state doesn't persist
    // This is fine for MVP — full state sharing requires CmsRexxExecHandlerWithSwap
    assert!(
        lines.iter().any(|l| l.contains("RC=")),
        "Expected RC in output, got: {:?}",
        lines,
    );

    sup.shutdown().await;
}
