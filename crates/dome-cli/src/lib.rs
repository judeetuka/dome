use std::env;
use std::path::PathBuf;

use dome_host_linux::{HostReport, ProbeRoot};

pub fn run_from_env() -> Result<String, CliError> {
    touch_internal_marker();
    run(env::args().skip(1).collect())
}

fn touch_internal_marker() {
    // obfuscation: anti-strings hygiene only; not a security boundary; reason: release binary marker used for static-scan checks.
    let marker = dome_obfuscation::obfstr!("dome-internal-release-marker:v1");
    std::hint::black_box(marker);
}

pub fn run(args: Vec<String>) -> Result<String, CliError> {
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(help_text());
    }

    match args[0].as_str() {
        "doctor" => run_doctor(&args[1..]),
        command => Err(CliError::UnknownCommand(command.to_owned())),
    }
}

fn run_doctor(args: &[String]) -> Result<String, CliError> {
    let mut format = OutputFormat::Toml;
    let mut fixture_root = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--format" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::MissingValue("--format"));
                };
                format = OutputFormat::parse(value)?;
                index += 2;
            }
            "--fixtures" => {
                let Some(path) = args.get(index + 1) else {
                    return Err(CliError::MissingValue("--fixtures"));
                };
                fixture_root = Some(PathBuf::from(path));
                index += 2;
            }
            flag => return Err(CliError::UnknownFlag(flag.to_owned())),
        }
    }

    let report = match fixture_root {
        Some(root) => HostReport::probe_root(&ProbeRoot::new(root)),
        None => HostReport::probe_host(),
    };

    match format {
        OutputFormat::Toml => Ok(report.to_toml()),
        OutputFormat::Human => Ok(report.to_human_text()),
    }
}

fn help_text() -> String {
    concat!(
        "dome\n\n",
        "Commands:\n",
        "  doctor [--format toml|human] [--fixtures PATH]  ",
        "Report read-only host capability hints\n\n",
        "Milestone 1 is read-only. It does not create chambers, change networking, ",
        "bind devices, or require root."
    )
    .to_owned()
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum OutputFormat {
    Toml,
    Human,
}

impl OutputFormat {
    fn parse(value: &str) -> Result<Self, CliError> {
        match value {
            "toml" => Ok(Self::Toml),
            "human" => Ok(Self::Human),
            other => Err(CliError::UnknownFormat(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    #[error("missing value for {0}")]
    MissingValue(&'static str),
    #[error("unknown command: {0}")]
    UnknownCommand(String),
    #[error("unknown flag: {0}")]
    UnknownFlag(String),
    #[error("unknown output format: {0}")]
    UnknownFormat(String),
}
