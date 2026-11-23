use std::env;
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use dome_config::Blueprint;
use dome_host_linux::{HostReport, ProbeRoot};

pub mod chambers;
pub mod foundation_build;
pub mod kernel;
pub mod packaging;
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

    /// Launch a chamber from a blueprint and return
    Run {
        #[arg(value_name = "CELL_TOML")]
        cell: PathBuf,
        /// Chamber name; defaults to the blueprint's file stem
        #[arg(long, value_name = "ID")]
        name: Option<String>,
        /// Kernel override; the foundation supplies one otherwise
        #[arg(long, value_name = "BZIMAGE")]
        kernel: Option<PathBuf>,
        /// Initramfs override
        #[arg(long, value_name = "IMAGE")]
        initramfs: Option<PathBuf>,
        /// Guest memory in MiB
        #[arg(long, value_name = "MIB", default_value_t = 64)]
        memory_mib: u64,
        /// Wall-clock limit for the chamber, in seconds
        #[arg(long, value_name = "SECONDS", default_value_t = 3600)]
        timeout: u64,
        /// State directory; defaults to the user's state directory
        #[arg(long, value_name = "DIR")]
        state: Option<PathBuf>,
        /// Foundation search root
        #[arg(long, value_name = "DIR")]
        foundations: Option<PathBuf>,
    },

    /// Stop a running chamber
    Stop {
        #[arg(value_name = "CHAMBER_ID")]
        chamber_id: String,
        /// Seconds to wait after asking before insisting
        #[arg(long, value_name = "SECONDS", default_value_t = 10)]
        grace: u64,
        /// State directory
        #[arg(long, value_name = "DIR")]
        state: Option<PathBuf>,
    },

    /// List chambers
    Ps {
        /// State directory
        #[arg(long, value_name = "DIR")]
        state: Option<PathBuf>,
    },

    /// The chamber process itself; started by dome run, not by hand
    #[command(hide = true, name = "__boot")]
    Boot {
        #[arg(long, value_name = "CELL_TOML")]
        cell: PathBuf,
        #[arg(long, value_name = "ID")]
        name: Option<String>,
        #[arg(long, value_name = "BZIMAGE")]
        kernel: Option<PathBuf>,
        #[arg(long, value_name = "IMAGE")]
        initramfs: Option<PathBuf>,
        #[arg(long, value_name = "MIB", default_value_t = 64)]
        memory_mib: u64,
        #[arg(long, value_name = "SECONDS", default_value_t = 3600)]
        timeout: u64,
        #[arg(long, value_name = "DIR")]
        state: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        foundations: Option<PathBuf>,
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

    /// Manage kernel profiles
    Kernel {
        #[command(subcommand)]
        action: KernelAction,
    },

    /// Generate the keys a release needs
    Keygen {
        /// Directory to write the keys into
        #[arg(long, value_name = "DIR", required = true)]
        out: PathBuf,
    },

    /// Package a release as a self-contained export
    Export {
        /// The artifact to package
        #[arg(value_name = "ARTIFACT")]
        artifact: PathBuf,
        /// Key directory that can open it
        #[arg(long, value_name = "DIR")]
        keys: PathBuf,
        /// Where to write the package
        #[arg(long, value_name = "PATH", required = true)]
        output: PathBuf,
    },

    /// Unpack an export into a usable artifact
    Import {
        /// The package to unpack
        #[arg(value_name = "PACKAGE")]
        package: PathBuf,
        /// Key directory that can open it
        #[arg(long, value_name = "DIR")]
        keys: PathBuf,
        /// Where to write the artifact
        #[arg(long, value_name = "PATH", required = true)]
        output: PathBuf,
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

    /// Build a foundation from a recipe
    Build {
        /// Recipe path; the file may include others
        #[arg(long, value_name = "TOML")]
        recipe: Option<PathBuf>,
        /// List the available recipe files in the workspace
        #[arg(long)]
        list: bool,
        /// A kernel image to use instead of the recipe's profile (development override)
        #[arg(long, value_name = "BZIMAGE", hide = true)]
        kernel: Option<PathBuf>,
        /// Where the foundation directory goes
        #[arg(long, value_name = "DIR")]
        output: Option<PathBuf>,
        /// The static init to run as pid 1
        #[arg(long, value_name = "PATH")]
        init: Option<PathBuf>,
        /// A program to embed and run
        #[arg(long, value_name = "PATH")]
        workload: Option<PathBuf>,
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
enum KernelAction {
    /// Install a kernel image under a profile name
    Import {
        /// Profile name, for example dome-minimal
        #[arg(long, value_name = "NAME")]
        profile: String,
        /// The kernel image to install
        #[arg(long, value_name = "BZIMAGE")]
        from: PathBuf,
    },

    /// List installed profiles
    List,
}

#[derive(Debug, Subcommand)]
enum FoundationAction {
    /// Read a .dome header without decrypting it
    Info {
        #[arg(value_name = "FILE_DOME")]
        file: PathBuf,
    },

    /// List the foundations in the store
    List {
        /// Store directory; defaults to the user's data directory
        #[arg(long, value_name = "DIR")]
        store: Option<PathBuf>,
    },

    /// Store an image or a chamber layer as a new foundation
    Commit {
        /// The image or layer to store
        #[arg(long, value_name = "PATH")]
        from: PathBuf,
        /// A human-readable name for the list
        #[arg(long, value_name = "NAME", required = true)]
        name: String,
        /// Target architecture the image was built for
        #[arg(long, value_name = "ARCH", default_value = "x86_64")]
        arch: String,
        /// Store directory
        #[arg(long, value_name = "DIR")]
        store: Option<PathBuf>,
    },

    /// Remove foundations no chamber references
    Prune {
        /// Store directory
        #[arg(long, value_name = "DIR")]
        store: Option<PathBuf>,
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

/// A size a human can read.
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Where chamber records and logs live.
fn state_root(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }

    if let Ok(base) = env::var("XDG_STATE_HOME") {
        return PathBuf::from(base).join("dome");
    }

    PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".to_owned()))
        .join(".local")
        .join("state")
        .join("dome")
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

        Commands::Run {
            cell,
            name,
            kernel,
            initramfs,
            memory_mib,
            timeout,
            state,
            foundations,
        } => {
            let mut options = chambers::RunOptions::new(&cell, state_root(state));
            options.name = name;
            options.kernel = kernel;
            options.initramfs = initramfs;
            options.memory_mib = memory_mib;
            options.timeout = std::time::Duration::from_secs(timeout);
            if let Some(root) = foundations {
                options.foundation_root = root;
            }

            chambers::start(&options).map_err(CliError::Failed)
        }

        Commands::Stop {
            chamber_id,
            grace,
            state,
        } => chambers::stop(
            &chamber_id,
            &state_root(state),
            std::time::Duration::from_secs(grace),
        )
        .map_err(CliError::Failed),

        Commands::Ps { state } => chambers::list(&state_root(state)).map_err(CliError::Failed),

        Commands::Boot {
            cell,
            name,
            kernel,
            initramfs,
            memory_mib,
            timeout,
            state,
            foundations,
        } => {
            let mut options = chambers::RunOptions::new(&cell, state_root(state));
            options.name = name;
            options.kernel = kernel;
            options.initramfs = initramfs;
            options.memory_mib = memory_mib;
            options.timeout = std::time::Duration::from_secs(timeout);
            if let Some(root) = foundations {
                options.foundation_root = root;
            }

            chambers::boot(&options).map_err(CliError::Failed)
        }
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

        Commands::Kernel { action } => match action {
            KernelAction::Import { profile, from } => {
                kernel::import(&kernel::kernel_root(), &profile, &from).map_err(CliError::Failed)
            }
            KernelAction::List => {
                let profiles = kernel::list(&kernel::kernel_root()).map_err(CliError::Failed)?;
                if profiles.is_empty() {
                    Ok("no kernel profiles installed".to_owned())
                } else {
                    Ok(profiles
                        .into_iter()
                        .map(|profile| format!("{profile}\n"))
                        .collect())
                }
            }
        },

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

        Commands::Export {
            artifact,
            keys,
            output,
        } => packaging::export(&artifact, &release::KeyFiles::in_dir(&keys), &output)
            .map_err(CliError::Failed),

        Commands::Import {
            package,
            keys,
            output,
        } => packaging::import(&package, &release::KeyFiles::in_dir(&keys), &output)
            .map_err(CliError::Failed),

        Commands::Build {
            recipe,
            list,
            kernel,
            output,
            init,
            workload,
        } => {
            if list {
                return Ok(
                    "recipe files are discovered in the workspace; pass --recipe <path>".to_owned(),
                );
            }

            let (Some(recipe), Some(output)) = (recipe, output) else {
                return Err(CliError::Usage(
                    "build needs --recipe <toml> and --output <dir>; the recipe names its kernel profile"
                        .to_owned(),
                ));
            };

            foundation_build::build(&foundation_build::BuildOptions {
                recipe,
                kernel,
                kernel_root: kernel::kernel_root(),
                output,
                init,
                workload,
            })
            .map_err(CliError::Failed)
        }

        Commands::Inspect { artifact, keys } => {
            release::inspect(&artifact, &release::KeyFiles::in_dir(&keys)).map_err(CliError::Failed)
        }
        Commands::Foundation { action } => match action {
            FoundationAction::Info { .. } => Err(CliError::NotYetImplemented("foundation info")),

            FoundationAction::List { store } => {
                let store = dome_image::store::FoundationStore::at(
                    store.unwrap_or_else(dome_image::store::FoundationStore::default_root),
                );

                let entries = store
                    .list()
                    .map_err(|error| CliError::Failed(error.to_string()))?;
                if entries.is_empty() {
                    return Ok("no foundations stored".to_owned());
                }

                let mut report = String::from(
                    "DIGEST                                                            ARCH      SIZE        REFS  NAME\n",
                );
                for entry in entries {
                    report.push_str(&format!(
                        "{}  {:<9} {:<11} {:<5} {}\n",
                        entry.digest,
                        entry.arch,
                        format_size(entry.size),
                        entry.references.len(),
                        entry.name
                    ));
                }
                Ok(report)
            }

            FoundationAction::Commit {
                from,
                name,
                arch,
                store,
            } => {
                let store = dome_image::store::FoundationStore::at(
                    store.unwrap_or_else(dome_image::store::FoundationStore::default_root),
                );

                let entry = store
                    .commit(&from, &arch, &name)
                    .map_err(|error| CliError::Failed(error.to_string()))?;

                Ok(format!(
                    "stored {}\n  digest: {}\n  arch: {}\n  size: {}\n",
                    name,
                    entry.digest,
                    entry.arch,
                    format_size(entry.size)
                ))
            }

            FoundationAction::Prune { store } => {
                let store = dome_image::store::FoundationStore::at(
                    store.unwrap_or_else(dome_image::store::FoundationStore::default_root),
                );

                let removed = store
                    .prune()
                    .map_err(|error| CliError::Failed(error.to_string()))?;
                Ok(if removed.is_empty() {
                    "nothing to prune".to_owned()
                } else {
                    format!("removed {} foundation(s)", removed.len())
                })
            }
        },
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
            // A chamber or release that could not be produced is an engine or environment
            // failure, which the CLI contract keeps distinct from misusing the command line.
            Self::Failed(_) => 5,
            _ => 2,
        }
    }
}
