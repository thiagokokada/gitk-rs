//! Small UI preferences, separate from repository/history data.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub ui_font: String,
    pub fixed_font: String,
    pub font_size: f32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            ui_font: if cfg!(target_os = "macos") {
                ".SystemUIFont"
            } else {
                "DejaVu Sans"
            }
            .into(),
            fixed_font: if cfg!(target_os = "macos") {
                "Menlo"
            } else {
                "DejaVu Sans Mono"
            }
            .into(),
            font_size: 13.,
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
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
            .join("gitk-rs/preferences.json"))
    }
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path()?)
    }
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path()?)
    }
    fn load_from(path: &Path) -> Result<Self> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 65536, "preferences exceed 64 KiB");
        let preferences: Self = serde_json::from_slice(&bytes)?;
        preferences.validate()?;
        Ok(preferences)
    }
    fn save_to(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path.parent().context("preferences directory is missing")?;
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&serde_json::to_vec_pretty(self)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_preferences_roundtrip_and_invalid_input() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config/preferences.json");
        let prefs = Preferences {
            fixed_font: "Example Mono".into(),
            font_size: 16.,
            ..Preferences::default()
        };
        prefs.save_to(&path).unwrap();
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
}
