use std::process::ExitCode;

use grove::bootstrap;

fn main() -> ExitCode {
    bootstrap::install_panic_hook();
    match bootstrap::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
