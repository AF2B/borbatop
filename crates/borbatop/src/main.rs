//! Borba top: a native Linux task manager for the terminal, made for Arch.

mod cli;
mod config;
mod convert;
mod model;
mod recording;
mod reports;
mod run;
mod snapshot;
mod worker;

use std::error::Error;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Extra, write_completions, write_man_page};
use crate::config::{FileConfig, Settings, default_path, load};

fn main() -> ExitCode {
    match start() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("borbatop: {error}");
            ExitCode::FAILURE
        }
    }
}

fn start() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match &cli.command {
        Some(Extra::Completions { shell }) => {
            write_completions(*shell, &mut io::stdout());
            return Ok(());
        }
        Some(Extra::Man) => return Ok(write_man_page(&mut io::stdout())?),
        Some(Extra::Snapshot { .. } | Extra::Record { .. } | Extra::Replay { .. }) | None => {}
    }

    let path = cli.config.clone().or_else(|| {
        let xdg = std::env::var("XDG_CONFIG_HOME").ok();
        let home = std::env::var("HOME").ok();
        default_path(xdg.as_deref(), home.as_deref())
    });
    let file = match path {
        Some(path) => load(&path)?,
        None => FileConfig::default(),
    };
    let settings = Settings::resolve(&file, cli.overrides())?;

    if let Some(snapshot_command @ Extra::Snapshot { .. }) = &cli.command {
        let json = snapshot::capture(settings, snapshot_command.command_lines())?;
        writeln!(io::stdout(), "{}", json.render())?;
        return Ok(());
    }

    match &cli.command {
        Some(Extra::Record {
            file,
            samples,
            with_commands,
        }) => {
            let commands = Extra::command_lines_for(*with_commands);
            recording::record(settings, file, *samples, commands)?;
            eprintln!("recorded {samples} samples to {}", file.display());
            return Ok(());
        }
        Some(Extra::Replay { file }) => {
            let models = recording::load(file, settings.history_len)?;
            run::replay(settings, models)?;
            return Ok(());
        }
        Some(Extra::Completions { .. } | Extra::Man | Extra::Snapshot { .. }) | None => {}
    }

    if !io::stdout().is_terminal() {
        return Err("needs an interactive terminal: standard output is not a terminal".into());
    }
    run::run(settings)?;
    Ok(())
}
