use std::env;
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use dome_config::Blueprint;
use dome_host_linux::{HostReport, ProbeRoot};

pub mod release;

/// Dome command-line entry point.
#[derive(Debug, Parser)]
#[command(
    name = "dome",
    version,
    about = "Native-hypervisor sandbox engine. No QEMU, no WSL.",
    after_help = "Milestone 1 is read-only. It does not create chambers, change networking, \
                  bind devices, or require root."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Report read-only host capability hints
    Doctor {
        /// Output format
        #[arg(long, value_enum, default_value = "toml")]
        format: OutputFormat,
        /// Read probes from a fake host root instead of /
        #[arg(long, value_name = "PATH")]
        fixtures: Option<PathBuf>,
    },

    /// Parse and validate a cell blueprint
    Validate {
        /// Path to a .cell.toml blueprint
        #[arg(value_name = "CELL_TOML")]
        cell: PathBuf,
        /// Output format
        #[arg(long, value_enum, default_value = "toml")]
        format: OutputFormat,
    },

    /// Launch a chamber from a blueprint (arrives in M4)
    Run {
        #[arg(value_name = "CELL_TOML")]
        cell: PathBuf,
        /// Chamber id; defaults to the blueprint's foundation-adjacent name
        #[arg(long, value_name = "ID")]
        name: Option<String>,
        /// Display backend
        #[arg(long, value_name = "MODE")]
        display: Option<String>,
        /// VNC port; only meaningful with --display vnc
        #[arg(long, value_name = "PORT")]
        vnc_port: Option<u16>,
    },

    /// Stop a running chamber (arrives in M4)
    Stop {
        #[arg(value_name = "CHAMBER_ID")]
        chamber_id: String,
    },

    /// Build a release artifact from a cell blueprint
    Release {
        /// Path to the cell blueprint, which is embedded in the artifact
        #[arg(long, value_name = "CELL_TOML")]
        cell: PathBuf,
        /// Kernel override; otherwise the foundation named by the cell supplies it
        #[arg(long, value_name = "BZIMAGE")]
        kernel: Option<PathBuf>,
        /// Optional initramfs
        #[arg(long, value_name = "IMAGE")]
        initramfs: Option<PathBuf>,
        /// Guest memory in MiB for the capture boot
        #[arg(long, value_name = "MIB", default_value_t = 64)]
        memory_mib: u64,
        /// Key directory from dome keygen
        #[arg(long, value_name = "DIR")]
        keys: PathBuf,
        /// Where to write the artifact
        #[arg(long, value_name = "PATH", required = true)]
        output: PathBuf,
        /// Seal the release: it will run only on hardware that keeps guest memory confidential
        #[arg(long)]
        seal: bool,
        /// Declare the protection this device offers while capturing
        #[arg(long, value_name = "TIER", default_value = "none")]
        captured_on: String,
    },

    /// Generate the keys a release needs
    Keygen {
        /// Directory to write the keys into
        #[arg(long, value_name = "DIR", required = true)]
        out: PathBuf,
    },

    /// Verify a release and report whether this device can run it
    Inspect {
        /// Path to the artifact
        #[arg(value_name = "ARTIFACT")]
        artifact: PathBuf,
        /// Key directory that can open it
        #[arg(long, value_name = "DIR")]
        keys: PathBuf,
    },

    /// Build a foundation from a recipe (arrives in M5)
    Build {
        /// Recipe name, for example buildroot-minimal
        #[arg(long, value_name = "NAME", conflicts_with = "list")]
        recipe: Option<String>,
        /// List the available recipes
        #[arg(long)]
        list: bool,
    },

    /// Print the header of a .dome artifact (arrives in M2)
    Foundation {
        #[command(subcommand)]
        action: FoundationAction,
    },

    /// Generate shell completions
    #[command(hide = true)]
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Subcommand)]
enum FoundationAction {
    /// Read a .dome header without decrypting it
    Info {
        #[arg(value_name = "FILE_DOME")]
        file: PathBuf,
    },
}

/// Output format for machine-readable or human-readable text.
#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Toml,
    Human,
}

pub fn run_from_env() -> Result<String, CliError> {
    touch_internal_marker();
    run(env::args().skip(1).collect())
}

fn touch_internal_marker() {
    // obfuscation: anti-strings hygiene only; not a security boundary; reason: release binary marker used for static-scan checks.
    let marker = dome_obfuscation::obfstr!("dome-internal-release-marker:v1");
    std::hint::black_box(marker);
}

/// Parse arguments and dispatch. Kept synchronous and panicking-free so it is
/// directly testable without spawning a process.
pub fn run(args: Vec<String>) -> Result<String, CliError> {
    let argv = std::iter::once("dome".to_owned()).chain(args);

    match Cli::try_parse_from(argv) {
        Ok(cli) => match cli.command {
            Some(command) => dispatch(command),
            None => Ok(help_text()),
        },
        // --help and --version are successful outcomes, not errors.
        Err(error) => match error.kind() {
            clap::error::ErrorKind::DisplayHelp
            | clap::error::ErrorKind::DisplayVersion
            | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                Ok(error.render().to_string())
            }
            _ => Err(CliError::Usage(error.render().to_string())),
        },
    }
}

fn help_text() -> String {
    let mut command = Cli::command();
    let mut buffer = Vec::new();

    if command.write_long_help(&mut buffer).is_err() {
        return "dome\n".to_owned();
    }

    String::from_utf8_lossy(&buffer).into_owned()
}

fn dispatch(command: Commands) -> Result<String, CliError> {
    match command {
        Commands::Doctor { format, fixtures } => Ok(run_doctor(format, fixtures)),

        Commands::Validate { cell, format } => run_validate(&cell, format),

        Commands::Completions { shell } => {
            let mut buffer = Vec::new();
            clap_complete::generate(shell, &mut Cli::command(), "dome", &mut buffer);
            Ok(String::from_utf8_lossy(&buffer).into_owned())
        }

        Commands::Run { .. } => Err(CliError::NotYetImplemented("run")),
        Commands::Stop { .. } => Err(CliError::NotYetImplemented("stop")),
        Commands::Release {
            cell,
            kernel,
            initramfs,
            memory_mib,
            keys,
            output,
            seal,
            captured_on,
        } => {
            let Some(protection) = release::parse_protection(&captured_on) else {
                return Err(CliError::Usage(format!(
                    "unknown protection tier {captured_on:?}; try none, tdx, sev-snp or shielded-vm"
                )));
            };

            let files = release::KeyFiles::in_dir(&keys);
            let mut options = release::ReleaseOptions::new(&cell, &output, files);
            options.kernel = kernel;
            options.initramfs = initramfs;
            options.memory_mib = memory_mib;
            options.sealed = seal;
            options.captured_on = protection;

            release::release(&options).map_err(CliError::Failed)
        }

        Commands::Keygen { out } => {
            let files = release::KeyFiles::in_dir(&out);
            files.generate().map_err(CliError::Failed)?;
            files.seal_permissions().map_err(CliError::Failed)?;

            Ok(format!(
                "wrote keys to {}\n  identity: {}\n  signing: {}\n  mldsa: {}",
                files.dir.display(),
                files.identity.display(),
                files.signing.display(),
                files.mldsa.display()
            ))
        }

        Commands::Inspect { artifact, keys } => {
            release::inspect(&artifact, &release::KeyFiles::in_dir(&keys)).map_err(CliError::Failed)
        }
        Commands::Build { .. } => Err(CliError::NotYetImplemented("build")),
        Commands::Foundation { .. } => Err(CliError::NotYetImplemented("foundation")),
    }
}

fn run_doctor(format: OutputFormat, fixtures: Option<PathBuf>) -> String {
    let report = match fixtures {
        Some(root) => HostReport::probe_root(&ProbeRoot::new(root)),
        None => HostReport::probe_host(),
    };

    match format {
        OutputFormat::Toml => report.to_toml(),
        OutputFormat::Human => report.to_human_text(),
    }
}

fn run_validate(path: &Path, format: OutputFormat) -> Result<String, CliError> {
    let source = std::fs::read_to_string(path).map_err(|error| CliError::UnreadableFile {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;

    let blueprint = match Blueprint::from_toml(&source) {
        Ok(blueprint) => blueprint,
        Err(error) => {
            return Err(CliError::InvalidBlueprint {
                source_path: path.display().to_string(),
                count: 1,
                messages: format!("- {error}"),
            });
        }
    };

    if let Err(errors) = blueprint.validate() {
        let messages = errors
            .iter()
            .map(|error| format!("- {error}"))
            .collect::<Vec<_>>()
            .join("\n");

        return Err(CliError::InvalidBlueprint {
            source_path: path.display().to_string(),
            count: errors.len(),
            messages,
        });
    }

    Ok(render_valid_blueprint(path, &blueprint, format))
}

fn render_valid_blueprint(path: &Path, blueprint: &Blueprint, format: OutputFormat) -> String {
    let cell = blueprint.cell();
    let limits = cell.limits();
    let apertures = cell.apertures();

    match format {
        OutputFormat::Toml => format!(
            concat!(
                "[validation]\n",
                "valid = true\n",
                "source = \"{}\"\n",
                "foundation = \"{}\"\n",
                "memory_mib = {}\n",
                "storage_mib = {}\n",
                "cpus = {}\n",
                "published_ports = {}\n",
                "file_apertures = {}\n",
                "gpio_apertures = {}\n"
            ),
            path.display(),
            cell.foundation().as_str(),
            limits.memory().as_mib(),
            limits.storage().as_mib(),
            limits.cpus().get(),
            cell.ports().len(),
            apertures.files().len(),
            apertures.gpio().len(),
        ),
        OutputFormat::Human => format!(
            concat!(
                "blueprint: {}\n",
                "valid\n",
                "foundation: {}\n",
                "limits: {} MiB memory, {} MiB storage, {} cpu(s)\n",
                "perimeter: {} published port(s)\n",
                "apertures: {} file, {} gpio\n"
            ),
            path.display(),
            cell.foundation().as_str(),
            limits.memory().as_mib(),
            limits.storage().as_mib(),
            limits.cpus().get(),
            cell.ports().len(),
            apertures.files().len(),
            apertures.gpio().len(),
        ),
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
    #[error("{0}")]
    Usage(String),
    #[error("`{0}` is not implemented yet; see PLAN.md for the milestone that adds it")]
    NotYetImplemented(&'static str),
    #[error("cannot read {path}: {reason}")]
    UnreadableFile { path: String, reason: String },
    #[error("{0}")]
    Failed(String),

    #[error("{source_path}: {count} problem(s) found\n{messages}")]
    InvalidBlueprint {
        source_path: String,
        count: usize,
        messages: String,
    },
}

impl CliError {
    /// Process exit code for this error, per docs/cli-contract.md.
    ///
    /// 2 = usage, 3 = validation. Codes 4 (capability missing) and 5 (engine failure)
    /// are reserved for the engine work in M3/M4 and cannot be produced yet.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidBlueprint { .. } | Self::UnreadableFile { .. } => 3,
            _ => 2,
        }
    }
}
