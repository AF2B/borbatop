//! Settings: defaults, the optional TOML file and command-line overrides, validated once.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use borbatop_tui::ThemeKind;
use clap::ValueEnum;
use serde::Deserialize;

const DEFAULT_INTERVAL_MS: u64 = 1_000;
const MIN_INTERVAL_MS: u64 = 200;
const MAX_INTERVAL_MS: u64 = 10_000;
const DEFAULT_HISTORY: usize = 60;
const MIN_HISTORY: usize = 8;
const MAX_HISTORY: usize = 600;
const CONFIG_DIR: &str = "borbatop";
const CONFIG_FILE: &str = "config.toml";
const USER_CONFIG_DIR: &str = ".config";

/// Palette names accepted in the file and on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    /// The Borba palette in 24-bit color.
    Borba,
    /// The terminal's own 16 colors.
    Ansi,
}

impl ThemeName {
    const fn kind(self) -> ThemeKind {
        match self {
            Self::Borba => ThemeKind::Borba,
            Self::Ansi => ThemeKind::Ansi,
        }
    }
}

/// Contents of `config.toml`; every key is optional and unknown keys are rejected.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    interval_ms: Option<u64>,
    theme: Option<ThemeName>,
    tree: Option<bool>,
    show_package: Option<bool>,
    history: Option<usize>,
}

/// Values given on the command line, which win over the file.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Overrides {
    /// `--interval-ms`.
    pub interval_ms: Option<u64>,
    /// `--theme`.
    pub theme: Option<ThemeName>,
    /// `--tree`.
    pub tree: bool,
    /// `--no-package`.
    pub hide_package: bool,
}

/// Validated settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Time between samples.
    pub interval: Duration,
    /// Palette.
    pub theme: ThemeKind,
    /// Start in tree mode.
    pub tree: bool,
    /// Show the owning package column.
    pub show_package: bool,
    /// Samples kept for sparklines.
    pub history_len: usize,
}

/// Why the settings could not be loaded.
#[derive(Debug)]
pub enum ConfigError {
    /// The file exists but cannot be read.
    Read {
        /// File that failed.
        path: PathBuf,
        /// Underlying OS error.
        source: io::Error,
    },
    /// The file is not valid TOML or has unknown keys or wrong types.
    Parse {
        /// File that failed.
        path: PathBuf,
        /// Parser message, with line and column.
        source: toml::de::Error,
    },
    /// A number is outside the range the program supports.
    OutOfRange {
        /// Setting name.
        field: &'static str,
        /// Value given.
        value: u64,
        /// Smallest accepted value.
        min: u64,
        /// Largest accepted value.
        max: u64,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "cannot read {}: {source}", path.display())
            }
            Self::Parse { path, source } => {
                write!(formatter, "invalid {}: {source}", path.display())
            }
            Self::OutOfRange {
                field,
                value,
                min,
                max,
            } => write!(formatter, "{field} = {value} is outside {min}..={max}"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::OutOfRange { .. } => None,
        }
    }
}

/// Where the configuration file lives: `$XDG_CONFIG_HOME/borbatop/config.toml`, or
/// `~/.config/borbatop/config.toml` when `XDG_CONFIG_HOME` is unset or empty.
///
/// # Arguments
///
/// * `xdg_config_home` - value of `XDG_CONFIG_HOME`
/// * `home` - value of `HOME`
///
/// # Returns
///
/// `None` when neither variable is usable.
#[must_use]
pub fn default_path(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|value| !value.is_empty()) {
        Some(xdg) => PathBuf::from(xdg),
        None => Path::new(home.filter(|value| !value.is_empty())?).join(USER_CONFIG_DIR),
    };
    Some(base.join(CONFIG_DIR).join(CONFIG_FILE))
}

/// Reads the configuration file; a file that does not exist means all defaults.
///
/// # Arguments
///
/// * `path` - file to read
///
/// # Errors
///
/// * [`ConfigError::Read`] when the file exists but cannot be read.
/// * [`ConfigError::Parse`] when it is not valid for [`FileConfig`].
pub fn load(path: &Path) -> Result<FileConfig, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(FileConfig::default()),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    parse(&text, path)
}

fn parse(text: &str, path: &Path) -> Result<FileConfig, ConfigError> {
    toml::from_str(text).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

fn within(field: &'static str, value: u64, min: u64, max: u64) -> Result<u64, ConfigError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(ConfigError::OutOfRange {
            field,
            value,
            min,
            max,
        })
    }
}

impl Settings {
    /// Combines defaults, the file and the command line, in that order of priority.
    ///
    /// # Arguments
    ///
    /// * `file` - contents of the configuration file
    /// * `overrides` - values given on the command line
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::OutOfRange`] for an interval outside 200..=10000 ms or a history
    /// outside 8..=600 samples.
    pub fn resolve(file: &FileConfig, overrides: Overrides) -> Result<Self, ConfigError> {
        let interval_ms = within(
            "interval_ms",
            overrides
                .interval_ms
                .or(file.interval_ms)
                .unwrap_or(DEFAULT_INTERVAL_MS),
            MIN_INTERVAL_MS,
            MAX_INTERVAL_MS,
        )?;
        let history = within(
            "history",
            u64::try_from(file.history.unwrap_or(DEFAULT_HISTORY)).unwrap_or(u64::MAX),
            u64::try_from(MIN_HISTORY).unwrap_or(0),
            u64::try_from(MAX_HISTORY).unwrap_or(u64::MAX),
        )?;

        Ok(Self {
            interval: Duration::from_millis(interval_ms),
            theme: overrides
                .theme
                .or(file.theme)
                .map_or(ThemeKind::Borba, ThemeName::kind),
            tree: overrides.tree || file.tree.unwrap_or(false),
            show_package: !overrides.hide_package && file.show_package.unwrap_or(true),
            history_len: usize::try_from(history).unwrap_or(DEFAULT_HISTORY),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn file(text: &str) -> FileConfig {
        parse(text, Path::new("config.toml")).unwrap()
    }

    #[test]
    fn defaults_apply_without_a_file() {
        let settings = Settings::resolve(&FileConfig::default(), Overrides::default()).unwrap();
        assert_eq!(settings.interval, Duration::from_millis(1_000));
        assert_eq!(settings.theme, ThemeKind::Borba);
        assert!(!settings.tree);
        assert!(settings.show_package);
        assert_eq!(settings.history_len, 60);
    }

    #[test]
    fn the_file_overrides_defaults_and_the_command_line_overrides_the_file() {
        let config = file(
            "interval_ms = 2000\ntheme = \"ansi\"\ntree = true\nshow_package = false\nhistory = 120\n",
        );
        let from_file = Settings::resolve(&config, Overrides::default()).unwrap();
        assert_eq!(from_file.interval, Duration::from_secs(2));
        assert_eq!(from_file.theme, ThemeKind::Ansi);
        assert!(from_file.tree);
        assert!(!from_file.show_package);
        assert_eq!(from_file.history_len, 120);

        let overridden = Settings::resolve(
            &config,
            Overrides {
                interval_ms: Some(500),
                theme: Some(ThemeName::Borba),
                tree: false,
                hide_package: false,
            },
        )
        .unwrap();
        assert_eq!(overridden.interval, Duration::from_millis(500));
        assert_eq!(overridden.theme, ThemeKind::Borba);
        assert!(
            overridden.tree,
            "a flag cannot switch off what the file switched on"
        );
    }

    #[test]
    fn no_package_flag_hides_the_column() {
        let overrides = Overrides {
            hide_package: true,
            ..Overrides::default()
        };
        assert!(
            !Settings::resolve(&FileConfig::default(), overrides)
                .unwrap()
                .show_package
        );
    }

    #[test]
    fn out_of_range_numbers_are_rejected_with_the_field_name() {
        for (text, field) in [
            ("interval_ms = 10", "interval_ms"),
            ("interval_ms = 99999", "interval_ms"),
            ("history = 1", "history"),
            ("history = 9999", "history"),
        ] {
            let error = Settings::resolve(&file(text), Overrides::default()).unwrap_err();
            assert!(
                matches!(error, ConfigError::OutOfRange { field: name, .. } if name == field),
                "{text}"
            );
        }
        let cli = Overrides {
            interval_ms: Some(1),
            ..Overrides::default()
        };
        assert!(Settings::resolve(&FileConfig::default(), cli).is_err());
    }

    #[test]
    fn unknown_keys_and_wrong_types_are_parse_errors() {
        for text in [
            "colour = \"red\"",
            "interval_ms = \"fast\"",
            "theme = \"neon\"",
            "tree = 3",
            "[",
        ] {
            assert!(
                matches!(
                    parse(text, Path::new("c.toml")),
                    Err(ConfigError::Parse { .. })
                ),
                "{text}"
            );
        }
    }

    #[test]
    fn parse_errors_name_the_file_and_the_problem() {
        let message = parse("colour = 1", Path::new("/x/config.toml"))
            .unwrap_err()
            .to_string();
        assert!(message.contains("/x/config.toml"));
        assert!(message.contains("colour"));
    }

    #[test]
    fn config_path_follows_xdg_then_home() {
        assert_eq!(
            default_path(Some("/xdg"), Some("/home/a")),
            Some(PathBuf::from("/xdg/borbatop/config.toml"))
        );
        assert_eq!(
            default_path(None, Some("/home/a")),
            Some(PathBuf::from("/home/a/.config/borbatop/config.toml"))
        );
        assert_eq!(
            default_path(Some(""), Some("/home/a")),
            Some(PathBuf::from("/home/a/.config/borbatop/config.toml"))
        );
        assert_eq!(default_path(None, None), None);
        assert_eq!(default_path(None, Some("")), None);
    }

    #[test]
    fn a_missing_file_is_all_defaults_and_a_directory_is_a_read_error() {
        assert_eq!(
            load(Path::new("/nonexistent/borbatop.toml")).unwrap(),
            FileConfig::default()
        );
        assert!(matches!(
            load(Path::new("/tmp")),
            Err(ConfigError::Read { .. })
        ));
    }
}
