use std::process::ExitCode;

fn main() -> ExitCode {
    match dome_cli::run_from_env() {
        Ok(output) => {
            // Command output is printed as-is; a trailing newline is added only when
            // the payload does not already end with one (clap's help text does).
            println!("{}", output.trim_end());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(error.exit_code())
        }
    }
}
