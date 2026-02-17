mod app;
#[cfg(feature = "cms")]
mod cms_support;
mod input;
mod screen;

use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut app;
    #[allow(unused_mut)]
    let mut file_args_start = 1;

    #[cfg(feature = "cms")]
    {
        if args.len() >= 3 && args[1] == "--cms" {
            let base_path = &args[2];
            file_args_start = 3;
            match cms_support::setup_cms(base_path) {
                Ok((processor, cms_fs)) => {
                    app = app::App::with_cms(processor, cms_fs);
                }
                Err(e) => {
                    eprintln!("CMS setup error: {}", e);
                    process::exit(1);
                }
            }
        } else {
            app = app::App::new();
        }
    }

    #[cfg(not(feature = "cms"))]
    {
        app = app::App::new();
    }

    for file_arg in &args[file_args_start..] {
        if let Err(e) = app.load_file(file_arg) {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }

    if let Err(e) = app.run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}
