//! Command line: flags, and the `completions` and `man` subcommands.

use std::io::{self, Write};
use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use crate::config::{Overrides, ThemeName};
use crate::snapshot::CommandLines;

const BINARY_NAME: &str = "borbatop";

/// Native Linux task manager for the terminal, made for Arch and Arch-based distributions.
#[derive(Debug, Parser)]
#[command(name = "borbatop", version, about, long_about = None)]
pub struct Cli {
    /// Configuration file (default: `$XDG_CONFIG_HOME/borbatop/config.toml`).
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Time between samples, in milliseconds (200 to 10000).
    #[arg(short, long, value_name = "MS")]
    pub interval_ms: Option<u64>,

    /// Palette: the Borba truecolor palette or the terminal's 16 colors.
    #[arg(long, value_enum)]
    pub theme: Option<ThemeName>,

    /// Start with processes arranged as a tree.
    #[arg(short, long)]
    pub tree: bool,

    /// Hide the column that names the package owning each executable.
    #[arg(long)]
    pub no_package: bool,

    /// Leave the mouse to the terminal (no clicks or wheel; text selection works).
    #[arg(long)]
    pub no_mouse: bool,

    /// Extra commands.
    #[command(subcommand)]
    pub command: Option<Extra>,
}

/// Commands that print something and exit instead of opening the screen.
#[derive(Debug, Subcommand)]
pub enum Extra {
    /// Print a shell completion script to standard output.
    Completions {
        /// Shell to generate the script for.
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Print the manual page (roff) to standard output.
    Man,
    /// Take one sample, print it as JSON and exit. Meant for scripts and cron jobs.
    Snapshot {
        /// Include full command lines (they may contain secrets).
        #[arg(long)]
        with_commands: bool,
    },
}

impl Extra {
    /// Whether a snapshot includes command lines; commands that print no snapshot omit them.
    #[must_use]
    pub const fn command_lines(&self) -> CommandLines {
        match self {
            Self::Snapshot {
                with_commands: true,
            } => CommandLines::Include,
            Self::Snapshot {
                with_commands: false,
            }
            | Self::Completions { .. }
            | Self::Man => CommandLines::Omit,
        }
    }
}

impl Cli {
    /// The command-line values that override the configuration file.
    #[must_use]
    pub const fn overrides(&self) -> Overrides {
        Overrides {
            interval_ms: self.interval_ms,
            theme: self.theme,
            tree: self.tree,
            hide_package: self.no_package,
            no_mouse: self.no_mouse,
        }
    }
}

/// Writes a completion script for `shell`.
///
/// # Arguments
///
/// * `shell` - shell to generate for
/// * `output` - where to write the script
pub fn write_completions(shell: Shell, output: &mut dyn Write) {
    clap_complete::generate(shell, &mut Cli::command(), BINARY_NAME, output);
}

/// Writes the manual page.
///
/// # Arguments
///
/// * `output` - where to write the roff source
///
/// # Errors
///
/// Returns the I/O error of `output`.
pub fn write_man_page(output: &mut dyn Write) -> io::Result<()> {
    clap_mangen::Man::new(Cli::command()).render(output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once(&BINARY_NAME).chain(arguments))
    }

    #[test]
    fn flags_become_overrides() {
        let cli = parse(&[
            "-i",
            "500",
            "--theme",
            "ansi",
            "--tree",
            "--no-package",
            "--no-mouse",
        ])
        .unwrap();
        assert_eq!(
            cli.overrides(),
            Overrides {
                interval_ms: Some(500),
                theme: Some(ThemeName::Ansi),
                tree: true,
                hide_package: true,
                no_mouse: true,
            }
        );
    }

    #[test]
    fn snapshot_subcommand_and_command_lines() {
        let plain = parse(&["snapshot"]).unwrap();
        let plain = plain.command.unwrap();
        assert!(matches!(plain, Extra::Snapshot { .. }));
        assert_eq!(plain.command_lines(), CommandLines::Omit);

        let with = parse(&["snapshot", "--with-commands"]).unwrap();
        assert_eq!(with.command.unwrap().command_lines(), CommandLines::Include);

        assert!(parse(&["--with-commands"]).is_err());
    }

    #[test]
    fn no_arguments_override_nothing() {
        assert_eq!(parse(&[]).unwrap().overrides(), Overrides::default());
    }

    #[test]
    fn bad_input_is_rejected() {
        assert!(parse(&["--theme", "neon"]).is_err());
        assert!(parse(&["--interval-ms", "fast"]).is_err());
        assert!(parse(&["--unknown"]).is_err());
        assert!(parse(&["completions", "klingon"]).is_err());
    }

    #[test]
    fn completions_name_the_binary_and_its_flags() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            let mut script = Vec::new();
            write_completions(shell, &mut script);
            let text = String::from_utf8(script).unwrap();
            assert!(text.contains("borbatop"), "{shell}");
            assert!(text.contains("interval-ms"), "{shell}");
        }
    }

    #[test]
    fn man_page_is_roff_with_the_options() {
        let mut page = Vec::new();
        write_man_page(&mut page).unwrap();
        let text = String::from_utf8(page).unwrap();
        assert!(text.contains(".TH borbatop"));
        assert!(text.contains("interval"));
        assert!(text.contains("completions"));
    }
}
