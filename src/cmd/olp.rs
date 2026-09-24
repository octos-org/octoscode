//! Discoverable CLI adapters for the existing Bash OLP tools.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use clap::{Args, Parser, Subcommand};
use eyre::{Result, WrapErr, eyre};

#[derive(Debug, Parser)]
#[command(
    name = "octoscode olp",
    about = "OLP tools (requires Bash and each script's dependencies)"
)]
pub struct OlpCli {
    /// Bash executable. Windows defaults to Git Bash when found on PATH via git.
    #[arg(long, global = true, value_name = "EXE")]
    bash: Option<PathBuf>,
    #[command(subcommand)]
    command: OlpCommand,
}

#[derive(Debug, Subcommand)]
enum OlpCommand {
    /// Initialize OLP files in an existing directory without overwriting them.
    Init(Project),
    /// Watch newly appended board lines for a literal token.
    Watch {
        board: PathBuf,
        token: String,
        #[arg(long, default_value_t = 20)]
        interval: u64,
        #[arg(long)]
        skip_signature: Vec<String>,
        /// Harvest evolution signals on each match, then keep watching.
        #[arg(long)]
        harvest: Option<PathBuf>,
    },
    /// Append stdin to a board under the existing flock lock.
    BoardAppend { board: PathBuf },
    /// Evolution board tools.
    Evo {
        #[command(subcommand)]
        command: EvoCommand,
    },
}

#[derive(Debug, Args)]
struct Project {
    #[arg(default_value = ".")]
    directory: PathBuf,
}

#[derive(Debug, Subcommand)]
enum EvoCommand {
    /// Collect friction signals using the existing harvest script.
    Harvest {
        #[command(flatten)]
        project: Project,
        #[arg(long)]
        dry_run: bool,
    },
    /// Generate the FLAW index.
    Index(Project),
    /// Print evolution diagnostics.
    Metrics {
        #[command(flatten)]
        project: Project,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        baseline: Option<PathBuf>,
        #[arg(long)]
        stall: Option<PathBuf>,
        #[arg(long)]
        stall_threshold: Option<u64>,
        #[arg(long)]
        now: Option<String>,
    },
}

// Embed companions together: init installs watch; watch calls harvest;
// harvest calls board-append; metrics/index import the Python library.
const SCRIPTS: &[(&str, &str)] = &[
    ("olp-init.sh", include_str!("../../scripts/olp-init.sh")),
    (
        "olp-watch-board.sh",
        include_str!("../../scripts/olp-watch-board.sh"),
    ),
    (
        "olp-board-append.sh",
        include_str!("../../scripts/olp-board-append.sh"),
    ),
    (
        "olp-evo-harvest.sh",
        include_str!("../../scripts/olp-evo-harvest.sh"),
    ),
    (
        "olp-evo-index.sh",
        include_str!("../../scripts/olp-evo-index.sh"),
    ),
    (
        "olp-evo-metrics.sh",
        include_str!("../../scripts/olp-evo-metrics.sh"),
    ),
    (
        "olp-evo-lib.py",
        include_str!("../../scripts/olp-evo-lib.py"),
    ),
];

fn bash_path(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit.or_else(|| std::env::var_os("OCTOSCODE_BASH").map(PathBuf::from)) {
        return path;
    }
    #[cfg(windows)]
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("git.exe").is_file() {
                for candidate in [dir.join("bash.exe"), dir.join("../bin/bash.exe")] {
                    if candidate.is_file() {
                        return candidate;
                    }
                }
            }
        }
    }
    PathBuf::from("bash")
}

// Git Bash accepts C:/ paths; preserve ordinary paths/bytes on Unix.
fn shell_path(path: &Path) -> OsString {
    #[cfg(windows)]
    {
        path.as_os_str().to_string_lossy().replace('\\', "/").into()
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().to_owned()
    }
}

pub fn run(cli: OlpCli) -> Result<i32> {
    let mut command = Command::new(bash_path(cli.bash));
    let mut args: Vec<OsString> = Vec::new();
    let script = match cli.command {
        OlpCommand::Init(project) => {
            if !project.directory.is_dir() {
                return Err(eyre!(
                    "OLP project directory does not exist: {}",
                    project.directory.display()
                ));
            }
            command.current_dir(project.directory);
            "olp-init.sh"
        }
        OlpCommand::BoardAppend { board } => {
            args.push(shell_path(&board));
            "olp-board-append.sh"
        }
        OlpCommand::Watch {
            board,
            token,
            interval,
            skip_signature,
            harvest,
        } => {
            args.extend([
                shell_path(&board),
                token.into(),
                "--interval".into(),
                interval.to_string().into(),
            ]);
            for signature in skip_signature {
                args.extend(["--skip-signature".into(), signature.into()]);
            }
            if let Some(repo) = harvest {
                args.extend(["--harvest".into(), shell_path(&repo)]);
            }
            "olp-watch-board.sh"
        }
        OlpCommand::Evo { command: evo } => match evo {
            EvoCommand::Index(project) => {
                args.push(shell_path(&project.directory));
                "olp-evo-index.sh"
            }
            EvoCommand::Harvest { project, dry_run } => {
                args.push(shell_path(&project.directory));
                if dry_run {
                    args.push("--dry-run".into());
                }
                "olp-evo-harvest.sh"
            }
            EvoCommand::Metrics {
                project,
                since,
                json,
                baseline,
                stall,
                stall_threshold,
                now,
            } => {
                args.push(shell_path(&project.directory));
                for (flag, value) in [
                    ("--since", since),
                    ("--now", now),
                    ("--stall-threshold", stall_threshold.map(|v| v.to_string())),
                ] {
                    if let Some(value) = value {
                        args.extend([flag.into(), value.into()]);
                    }
                }
                for (flag, value) in [("--baseline", baseline), ("--stall", stall)] {
                    if let Some(value) = value {
                        args.extend([flag.into(), shell_path(&value)]);
                    }
                }
                if json {
                    args.push("--json".into());
                }
                "olp-evo-metrics.sh"
            }
        },
    };
    let bundle = tempfile::Builder::new()
        .prefix("octoscode-olp-")
        .tempdir()?;
    for (name, source) in SCRIPTS {
        let path = bundle.path().join(name);
        std::fs::write(&path, source.replace("\r\n", "\n"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let status = command.arg("--").arg(shell_path(&bundle.path().join(script))).args(args)
        .status().wrap_err("Could not start Bash for OLP; install Bash (Git Bash on Windows), or set --bash / OCTOSCODE_BASH. WSL Bash needs Linux paths; run the Linux binary inside WSL.")?;
    // Keep the bundle alive until the script and its synchronous helpers finish.
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        Ok(status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)))
    }
    #[cfg(not(unix))]
    {
        Ok(status.code().unwrap_or(1))
    }
}
