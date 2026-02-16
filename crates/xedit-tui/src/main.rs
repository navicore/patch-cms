mod app;
mod input;
mod screen;

use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut app = app::App::new();

    if args.len() > 1 {
        if let Err(e) = app.load_file(&args[1]) {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }

    if let Err(e) = app.run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}
