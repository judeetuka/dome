use std::process::ExitCode;

fn main() -> ExitCode {
    match dome_cli::run_from_env() {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
