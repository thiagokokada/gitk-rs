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
            .join("gitk-rs/preferences.toml"))
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
        let preferences: Self = toml::from_str(std::str::from_utf8(&bytes)?)?;
        preferences.validate()?;
        Ok(preferences)
    }
    fn save_to(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path.parent().context("preferences directory is missing")?;
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(toml::to_string_pretty(self)?.as_bytes())?;
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
    fn toml_preferences_reject_invalid_values_and_encoding() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.toml");
        for contents in [
            b"font_size = 100.0".as_slice(),
            b"font_size = 'large'",
            b"ui_font = ''",
            b"\xff",
        ] {
            fs::write(&path, contents).unwrap();
            assert!(Preferences::load_from(&path).is_err());
        }
        fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(Preferences::load_from(&path).is_err());
    }
}
