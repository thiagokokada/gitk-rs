//! Saved application defaults, separate from repository/history data.
use crate::{
    cli::{Config, Theme},
    git::MAX_HISTORY_READERS,
    graph::MAX_GRAPH_LANES,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SYSTEM_UI_FONT: &str = "System UI";
pub const SYSTEM_MONOSPACE_FONT: &str = "System Monospace";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub ui_font: String,
    pub fixed_font: String,
    pub font_size: f32,
    pub theme: Theme,
    pub batch: Option<usize>,
    pub graph_columns: usize,
    pub history_readers: usize,
    pub text_graph: bool,
    pub syntax: bool,
    pub watch: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        let config = Config::default();
        Self {
            ui_font: SYSTEM_UI_FONT.into(),
            fixed_font: SYSTEM_MONOSPACE_FONT.into(),
            font_size: 13.,
            theme: config.theme,
            batch: config.batch,
            graph_columns: config.graph_columns,
            history_readers: config.history_readers,
            text_graph: config.text_graph,
            syntax: config.syntax,
            watch: config.watch,
        }
    }
}
impl Preferences {
    pub fn with_config(&self, config: &Config) -> Self {
        Self {
            theme: config.theme,
            batch: config.batch,
            graph_columns: config.graph_columns,
            history_readers: config.history_readers,
            text_graph: config.text_graph,
            syntax: config.syntax,
            watch: config.watch,
            ..self.clone()
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.batch.is_none_or(|batch| batch > 0),
            "history batch size must be positive"
        );
        ensure!(
            self.batch.is_none_or(|batch| i64::try_from(batch).is_ok()),
            "history batch size exceeds TOML's signed 64-bit integer range"
        );
        ensure!(
            (1..=MAX_GRAPH_LANES).contains(&self.graph_columns),
            "graph columns must be between 1 and {MAX_GRAPH_LANES}"
        );
        ensure!(
            (1..=MAX_HISTORY_READERS).contains(&self.history_readers),
            "history readers must be between 1 and {MAX_HISTORY_READERS}"
        );
        ensure!(
            (8.0..=24.0).contains(&self.font_size),
            "font size must be between 8 and 24 pixels"
        );
        for name in [&self.ui_font, &self.fixed_font] {
            ensure!(
                !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control),
                "font family must be a nonempty name up to 256 bytes"
            );
        }
        Ok(())
    }
    pub fn path() -> Result<PathBuf> {
        Ok(dirs::config_dir()
            .context("cannot locate configuration directory")?
            .join("gitk-rs/preferences.toml"))
    }
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path()?)
    }
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path()?)
    }
    pub fn load_from(path: &Path) -> Result<Self> {
        let preferences: Self = crate::settings_file::read_toml(path, "preferences exceed 64 KiB")?;
        preferences.validate()?;
        Ok(preferences)
    }
    pub fn save_to(&self, path: &Path) -> Result<()> {
        self.validate()?;
        crate::settings_file::write(path, toml::to_string_pretty(self)?.as_bytes())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn atomic_preferences_roundtrip_and_invalid_input() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config/preferences.toml");
        let prefs = Preferences {
            fixed_font: "Example Mono".into(),
            font_size: 16.,
            ..Preferences::default()
        };
        prefs.save_to(&path).unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("font_size = 16.0")
        );
        assert_eq!(Preferences::load_from(&path).unwrap(), prefs);
        fs::write(&path, "{invalid").unwrap();
        assert!(Preferences::load_from(&path).is_err());
        assert!(
            Preferences {
                font_size: f32::NAN,
                ..prefs
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn read_only_preferences_are_not_replaced_by_atomic_save() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.toml");
        let original = Preferences::default();
        original.save_to(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let permissions = fs::metadata(&path).unwrap().permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        let changed = Preferences {
            font_size: 19.,
            ..original.clone()
        };
        assert!(changed.save_to(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(Preferences::load_from(&path).unwrap(), original);
        fs::set_permissions(&path, permissions).unwrap();
        changed.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path).unwrap(), changed);
    }

    #[test]
    fn toml_preferences_support_comments_and_defaults() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.toml");
        assert_eq!(
            Preferences::load_from(&path).unwrap(),
            Preferences::default()
        );
        fs::write(&path, "# Custom font size\nfont_size = 18.0\n").unwrap();
        assert_eq!(
            Preferences::load_from(&path).unwrap(),
            Preferences {
                font_size: 18.,
                ..Preferences::default()
            }
        );
    }

    #[test]
    fn all_persistent_options_roundtrip_and_feed_startup_defaults() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.toml");
        let preferences = Preferences {
            theme: Theme::Light,
            batch: Some(73),
            graph_columns: 1024,
            history_readers: 8,
            text_graph: true,
            syntax: false,
            watch: false,
            ..Preferences::default()
        };
        preferences.save_to(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("timings") && !text.contains("verbose"));
        let loaded = Preferences::load_from(&path).unwrap();
        assert_eq!(loaded, preferences);
        let config = crate::cli::parse_with_preferences([], &loaded).unwrap();
        assert_eq!(loaded.with_config(&config), preferences);
        let mut unlimited = preferences;
        unlimited.batch = None;
        unlimited.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path).unwrap().batch, None);
    }

    #[test]
    fn toml_preferences_reject_invalid_values_and_encoding() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.toml");
        for contents in [
            b"font_size = 100.0".as_slice(),
            b"font_size = 'large'",
            b"ui_font = ''",
            b"batch = 0",
            b"history_readers = 0",
            b"history_readers = 65",
            b"history_readers = -1",
            b"batch = -1",
            b"graph_columns = 0",
            b"graph_columns = 1025",
            b"theme = 'blue'",
            b"\xff",
        ] {
            fs::write(&path, contents).unwrap();
            assert!(Preferences::load_from(&path).is_err());
        }
        fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(Preferences::load_from(&path).is_err());
        #[cfg(target_pointer_width = "64")]
        assert!(
            Preferences {
                batch: Some(usize::MAX),
                ..Preferences::default()
            }
            .validate()
            .is_err()
        );
    }
}
