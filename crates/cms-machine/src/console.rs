use std::io::{self, BufRead, Write};
use std::sync::mpsc;

use tokio::runtime::Handle;
use vm_iucv::machine_id::MachineId;
use vm_iucv::supervisor::Supervisor;

use crate::handler::CmsMachineHandler;

/// Drain and print output lines until the batch-done sentinel arrives or timeout.
fn drain_until_sentinel(output_rx: &mpsc::Receiver<String>, timeout: std::time::Duration) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            // Timeout — drain whatever is left without blocking
            while let Ok(line) = output_rx.try_recv() {
                if line == CmsMachineHandler::BATCH_DONE {
                    return;
                }
                println!("{}", line);
            }
            return;
        }
        match output_rx.recv_timeout(remaining) {
            Ok(line) if line == CmsMachineHandler::BATCH_DONE => return,
            Ok(line) => println!("{}", line),
            Err(mpsc::RecvTimeoutError::Timeout) => return,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Run the interactive CMS console.
///
/// Reads lines from stdin, sends them to the CMS machine handler via channels,
/// and prints output. Runs on the calling thread (blocking).
pub fn run_console(
    handle: &Handle,
    supervisor: &Supervisor,
    con_id: &MachineId,
    user_id: &MachineId,
    cmd_tx: mpsc::Sender<String>,
    output_rx: mpsc::Receiver<String>,
) {
    // Drain initial IPL output (logon banner, PROFILE EXEC output).
    // The handler sends BATCH_DONE after on_ipl completes.
    drain_until_sentinel(&output_rx, std::time::Duration::from_secs(10));

    let stdin = io::stdin();
    let reader = stdin.lock();

    println!("Ready;");
    io::stdout().flush().ok();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // EOF
        };

        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            println!("Ready;");
            io::stdout().flush().ok();
            continue;
        }

        // Check for LOGOFF before sending
        if trimmed.eq_ignore_ascii_case("LOGOFF") {
            handle.block_on(async {
                let _ = supervisor.logoff(user_id).await;
                supervisor.shutdown().await;
            });
            println!("LOGOFF AT");
            return;
        }

        // Send command to the handler
        if cmd_tx.send(trimmed).is_err() {
            eprintln!("CMS machine has shut down");
            return;
        }

        // Wake the USER machine by sending SMSG from $CON
        let wake_result = handle.block_on(async { supervisor.smsg(con_id, user_id, "CMD").await });

        if let Err(e) = wake_result {
            eprintln!("Failed to wake machine: {}", e);
            continue;
        }

        // Wait for batch-done sentinel. Timeout is a safety net only — the
        // sentinel is the real sync mechanism. Use a generous timeout so slow
        // REXX EXECs (e.g., copying many files) don't cause premature Ready;.
        drain_until_sentinel(&output_rx, std::time::Duration::from_secs(300));

        println!("Ready;");
        io::stdout().flush().ok();
    }
}
