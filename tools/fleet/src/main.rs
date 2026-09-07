mod backend;
mod command;
mod config;
mod import;

use anyhow::Result;
use backend::Backend;
use clap::{Parser, Subcommand};
use command::Invocation;
use config::Config;
use std::{env, path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(
    version,
    about = "Power controls for existing Lima and Azure dev machines"
)]
struct Cli {
    /// Config path (default: $XDG_CONFIG_HOME/fleet/workspaces.toml or ~/.config/fleet/workspaces.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Preview actions/imports (ls/check/import still query providers)
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List configured workspaces and their current power state
    #[command(visible_alias = "list")]
    Ls,
    /// Register existing machines as workspaces
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },
    /// Start an existing machine and print connection information
    Up { workspace: Option<String> },
    /// Stop a Lima VM or deallocate an Azure VM; never delete it
    Down { workspace: Option<String> },
    /// Open a shell using limactl or SSH
    Connect { workspace: Option<String> },
    /// Run a command on a machine: fleet exec [workspace] -- command args...
    Exec {
        workspace: Option<String>,
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<String>,
    },
    /// Check CLI availability and Azure login
    Check,
}

#[derive(Subcommand)]
enum ImportSource {
    /// Import all existing Lima instances; skip machines already registered
    Lima,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(err) => {
            eprintln!("fleet: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<i32> {
    let path = match cli.config {
        Some(path) => path,
        None => config::default_path()?,
    };
    let config = Config::load(&path)?;
    if let Commands::Import {
        source: ImportSource::Lima,
    } = cli.command
    {
        let machines = backend::lima_names()?;
        let added = import::lima(&path, &machines, cli.dry_run)?;
        for name in &added {
            println!(
                "{} {name}",
                if cli.dry_run {
                    "Would import"
                } else {
                    "Imported"
                }
            );
        }
        if added.is_empty() {
            println!("No new Lima machines to import.");
        } else {
            println!(
                "{} {} workspace(s) to {}.",
                if cli.dry_run { "Would add" } else { "Added" },
                added.len(),
                path.display()
            );
        }
        return Ok(0);
    }
    if config.workspace.is_empty() {
        println!(
            "No machines registered. Run `fleet import lima` or add a workspace to {}.",
            path.display()
        );
        if !matches!(cli.command, Commands::Check) {
            return Ok(if matches!(cli.command, Commands::Ls) {
                0
            } else {
                1
            });
        }
    }
    if matches!(cli.command, Commands::Check) {
        return Ok(check());
    }
    if matches!(cli.command, Commands::Ls) {
        return Ok(list(&config));
    }
    let name = match &cli.command {
        Commands::Up { workspace }
        | Commands::Down { workspace }
        | Commands::Connect { workspace }
        | Commands::Exec { workspace, .. } => workspace.as_deref(),
        _ => unreachable!(),
    };
    let ws = config.select(name, &env::current_dir()?)?;
    match cli.command {
        Commands::Up { .. } => {
            let code = ws.machine.start(cli.dry_run)?;
            if code == 0 {
                match backend::connection(ws, &[]) {
                    Ok(cmd) => println!("Connect: {}", cmd.display()),
                    Err(err) => println!("{err}"),
                }
            }
            Ok(code)
        }
        Commands::Down { .. } => ws.machine.stop(cli.dry_run),
        Commands::Connect { .. } => backend::connection(ws, &[])?.run(cli.dry_run),
        Commands::Exec { command, .. } => backend::connection(ws, &command)?.run(cli.dry_run),
        _ => unreachable!(),
    }
}

fn list(config: &Config) -> i32 {
    println!(
        "{:<18} {:<8} {:<22} {:<14} {:>8}  TAGS",
        "WORKSPACE", "BACKEND", "MACHINE", "STATE", "RATE/HR"
    );
    // Each provider lookup is independent; keep slow Azure CLI startup off the serial path.
    std::thread::scope(|scope| {
        let pending: Vec<_> = config
            .workspace
            .iter()
            .map(|ws| (ws, scope.spawn(|| ws.machine.status())))
            .collect();
        let mut code = 0;
        for (ws, job) in pending {
            let state = match job.join().expect("status worker panicked") {
                Ok(state) => state,
                Err(err) => {
                    eprintln!("{}: {err:#}", ws.name);
                    code = 1;
                    "error".into()
                }
            };
            let rate = ws
                .hourly
                .map(|r| format!("{r:.2}"))
                .unwrap_or_else(|| "-".into());
            println!(
                "{:<18} {:<8} {:<22} {:<14} {:>8}  {}",
                ws.name,
                ws.machine.backend(),
                ws.machine.name(),
                state,
                rate,
                ws.tags.join(",")
            );
        }
        code
    })
}

fn check() -> i32 {
    let mut code = 0;
    for (name, cmd, hint) in [
        (
            "Lima",
            Invocation::new("limactl", &["--version"]),
            "Install Lima and make limactl available on PATH.",
        ),
        (
            "Azure",
            Invocation::new(
                "az",
                &["account", "show", "--output", "json", "--only-show-errors"],
            ),
            "Install Azure CLI, then run az login (and az account set if needed).",
        ),
        (
            "SSH",
            Invocation::new("ssh", &["-V"]),
            "Install OpenSSH and make ssh available on PATH.",
        ),
    ] {
        match cmd.output() {
            Ok(_) => println!("{name:<8} ok"),
            Err(err) => {
                println!("{name:<8} unavailable — {hint}");
                eprintln!("{err:#}");
                code = 1;
            }
        }
    }
    code
}
