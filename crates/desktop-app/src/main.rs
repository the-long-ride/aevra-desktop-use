use desktop_app::cli::{parse, Command, USAGE};
use desktop_app::{doctor, gui, mcp_stdio};
use desktop_policy::paths::{config_file, native_config_dir};
use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let command = match parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let outcome = match command {
        Command::Version => {
            println!("aevra-desktop {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Help => {
            print!("{USAGE}");
            Ok(())
        }
        Command::Doctor { json } => {
            let backend = doctor::probe_backend(Duration::from_secs(10));
            let report = doctor::collect(&config_file(&native_config_dir()), backend);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_default()
                );
            } else {
                print!("{}", doctor::render_text(&report));
            }
            Ok(())
        }
        Command::Mcp => mcp_stdio::run(),
        Command::Run { port } => gui::run_with_port(port),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("aevra-desktop: {message}");
            ExitCode::FAILURE
        }
    }
}
