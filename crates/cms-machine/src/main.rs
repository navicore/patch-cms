use std::sync::mpsc;

use cms_core::minidisk::AccessMode;
use cms_core::CmsFileSystem;
use vm_iucv::handler::{MachineContext, MachineHandler};
use vm_iucv::machine_id::MachineId;
use vm_iucv::message::SmsgMessage;
use vm_iucv::supervisor::Supervisor;

use cms_machine::console;
use cms_machine::handler::CmsMachineHandler;

/// Dummy handler for the $CON pseudo-machine. Only exists to be woken.
struct ConHandler;

impl MachineHandler for ConHandler {
    fn on_smsg(&mut self, _ctx: &MachineContext, _msg: SmsgMessage) {
        // $CON does nothing — it exists only as a sender identity
    }
}

fn parse_args() -> (String, Option<String>) {
    let args: Vec<String> = std::env::args().collect();
    let mut userid = "USER".to_string();
    let mut disk_path = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--userid" => {
                if i + 1 < args.len() {
                    userid = args[i + 1].to_ascii_uppercase();
                    i += 2;
                } else {
                    eprintln!("--userid requires a value");
                    std::process::exit(1);
                }
            }
            "--disk" => {
                if i + 1 < args.len() {
                    disk_path = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("--disk requires a path");
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                println!("Usage: cms-machine [--userid NAME] [--disk PATH]");
                println!();
                println!("Options:");
                println!("  --userid NAME   Machine userid (default: USER)");
                println!("  --disk PATH     Path to A-disk directory");
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument: {}", other);
                std::process::exit(1);
            }
        }
    }

    (userid, disk_path)
}

fn main() {
    let (userid, disk_path) = parse_args();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let handle = rt.handle().clone();

    let con_id = MachineId::new("$CON").expect("Invalid $CON machine id");
    let user_id = MachineId::new(&userid).unwrap_or_else(|e| {
        eprintln!("Invalid userid '{}': {}", userid, e);
        std::process::exit(1);
    });

    // Set up CMS filesystem
    let mut filesystem = CmsFileSystem::new();
    if let Some(ref path) = disk_path {
        let disk_dir = std::path::Path::new(path);
        if !disk_dir.exists() {
            std::fs::create_dir_all(disk_dir).unwrap_or_else(|e| {
                eprintln!("Cannot create disk directory '{}': {}", path, e);
                std::process::exit(1);
            });
        }

        // Access A-disk
        let a_path = disk_dir.join("a");
        if !a_path.exists() {
            std::fs::create_dir_all(&a_path).unwrap_or_else(|e| {
                eprintln!("Cannot create A-disk directory: {}", e);
                std::process::exit(1);
            });
        }
        filesystem
            .access_disk('A', &a_path, AccessMode::ReadWrite)
            .unwrap_or_else(|e| {
                eprintln!("Cannot access A-disk: {}", e);
                std::process::exit(1);
            });

        // Auto-access additional disks from subdirectories (B-Z)
        for letter in 'B'..='Z' {
            let sub = disk_dir.join(letter.to_ascii_lowercase().to_string());
            if sub.is_dir() {
                let _ = filesystem.access_disk(letter, &sub, AccessMode::ReadOnly);
            }
        }
    }

    // Create channels
    let (cmd_tx, cmd_rx) = mpsc::channel::<String>();
    let (output_tx, output_rx) = mpsc::channel::<String>();

    // Create the EXEC handler
    #[cfg(feature = "rexx")]
    let exec_handler: Box<dyn cms_core::ExecHandler> =
        { Box::new(cms_machine::rexx_exec::CmsRexxExecHandler) };
    #[cfg(not(feature = "rexx"))]
    let exec_handler: Box<dyn cms_core::ExecHandler> = { Box::new(cms_core::NoExecHandler) };

    let handler = CmsMachineHandler::new(filesystem, exec_handler, cmd_rx, output_tx);

    // Boot machines
    handle.block_on(async {
        let supervisor = Supervisor::new();

        supervisor
            .ipl(&con_id, ConHandler)
            .await
            .expect("Failed to IPL $CON");

        supervisor
            .ipl(&user_id, handler)
            .await
            .expect("Failed to IPL user machine");

        // Run console on the main thread
        console::run_console(&handle, &supervisor, &con_id, &user_id, cmd_tx, output_rx);
    });
}
