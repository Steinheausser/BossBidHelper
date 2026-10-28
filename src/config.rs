//! Where the bid data lives, and how that choice is remembered.
//!
//! Resolution order: `--data-dir <path>` argument, `BOSS_DATA_DIR` environment variable,
//! the folder last chosen in the app, then the default OneDrive location, then `./data`.

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Default location, relative to the user's home folder.
const DEFAULT_RELATIVE_DIR: &[&str] = &[
    "OneDrive - Singapore Management University",
    "School Documents",
    "Bidding and School Guides",
    "Bidding",
    "Boss Overall Resultsa",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub data_dir: Option<PathBuf>,
}

impl Config {
    fn path() -> PathBuf {
        let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        p.push("smu_bid_analyser");
        p.push("config.json");
        p
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Saves the settings. A failure is logged to stderr, not returned, since the UI has no good
    /// place to show it. Use `try_save` to get the error.
    pub fn save(&self) {
        if let Err(e) = self.try_save() {
            eprintln!("Could not save settings to {}: {e}", Self::path().display());
        }
    }

    /// Writes the settings to the config file, returning why it could not be written.
    pub fn try_save(&self) -> std::io::Result<()> {
        self.write_to(&Self::path())
    }

    fn write_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

pub fn default_data_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let p = DEFAULT_RELATIVE_DIR.iter().fold(home, |acc, part| acc.join(part));
        if p.is_dir() {
            return p;
        }
    }
    PathBuf::from("data")
}

pub fn resolve_data_dir(cli: Option<&Path>, config: &Config) -> PathBuf {
    resolve_with(cli, std::env::var_os("BOSS_DATA_DIR"), config)
}

/// The precedence rules, with the environment variable passed in so tests can cover them without
/// touching the real environment.
fn resolve_with(cli: Option<&Path>, env: Option<OsString>, config: &Config) -> PathBuf {
    if let Some(p) = cli {
        return p.to_path_buf();
    }
    if let Some(p) = env.filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    if let Some(p) = config.data_dir.as_ref().filter(|p| p.is_dir()) {
        return p.clone();
    }
    default_data_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_precedence_is_cli_then_env_then_config_then_default() {
        let existing = std::env::temp_dir();
        let missing = existing.join("smu_bid_analyser_test_no_such_folder");
        let cli = PathBuf::from("from-cli");
        let env = OsString::from("from-env");
        let with_saved = Config { data_dir: Some(existing.clone()) };

        // The command-line argument beats everything else.
        assert_eq!(resolve_with(Some(&cli), Some(env.clone()), &with_saved), cli);
        assert_eq!(resolve_with(Some(&cli), None, &Config::default()), cli);
        // Then the environment variable beats the saved folder.
        assert_eq!(resolve_with(None, Some(env.clone()), &with_saved), PathBuf::from(env.clone()));
        // Then the saved folder, as long as it still exists.
        assert_eq!(resolve_with(None, None, &with_saved), existing);
        // An empty environment variable counts as unset.
        assert_eq!(resolve_with(None, Some(OsString::new()), &with_saved), existing);
        // A saved folder that has gone missing falls through to the default.
        let stale = Config { data_dir: Some(missing) };
        assert_eq!(resolve_with(None, None, &stale), default_data_dir());
        assert_eq!(resolve_with(None, None, &Config::default()), default_data_dir());
    }

    #[test]
    fn write_to_reports_failures_instead_of_hiding_them() {
        let base = std::env::temp_dir().join(format!("smu_bid_analyser_config_test_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let config = Config { data_dir: Some(PathBuf::from("C:/Some/Folder")) };

        let file = base.join("config.json");
        config.write_to(&file).unwrap();
        let back: Config = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back.data_dir, config.data_dir);

        // A path whose parent is a regular file cannot be created, and the error must come back.
        let blocker = base.join("blocker");
        std::fs::write(&blocker, "not a folder").unwrap();
        assert!(config.write_to(&blocker.join("config.json")).is_err());

        std::fs::remove_dir_all(&base).unwrap();
    }
}
