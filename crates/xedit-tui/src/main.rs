mod app;
mod input;
mod screen;

use std::path::Path;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut app = app::App::new();

    if args.len() > 1 {
        let path = Path::new(&args[1]);
        if let Err(e) = app.load_file(path) {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }

    if let Err(e) = app.run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}
