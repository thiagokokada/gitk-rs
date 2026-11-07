//! Automatically saved diff controls, independent of explicitly applied preferences.
use super::DiffOptions;
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct State {
    diff: DiffOptions,
}

pub fn path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("cannot locate configuration directory")?
        .join("gitk-rs/state.toml"))
}

pub fn load_from(path: &Path) -> Result<DiffOptions> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DiffOptions::default());
        }
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "diff options exceed 64 KiB");
    let options = toml::from_str::<State>(std::str::from_utf8(&bytes)?)?.diff;
    ensure!(
        options.context <= 10_000_000,
        "diff context exceeds ten million lines"
    );
    Ok(options)
}

pub fn save_to(options: DiffOptions, path: &Path) -> Result<()> {
    ensure!(
        options.context <= 10_000_000,
        "diff context exceeds ten million lines"
    );
    crate::settings_file::write(
        path,
        toml::to_string_pretty(&State { diff: options })?.as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffEngine, DiffVersion, WordDiff};

    #[test]
    fn separate_file_roundtrips_defaults_and_rejects_bad_input() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config/state.toml");
        assert_eq!(load_from(&path).unwrap(), DiffOptions::default());
        for version in [DiffVersion::Diff, DiffVersion::Old, DiffVersion::New] {
            for words in [WordDiff::Lines, WordDiff::Markup, WordDiff::Color] {
                let options = DiffOptions {
                    version,
                    words,
                    context: 0,
                    ignore_space: true,
                };
                save_to(options, &path).unwrap();
                assert_eq!(load_from(&path).unwrap(), options);
            }
        }
        assert!(!path.with_file_name("preferences.toml").exists());
        fs::write(&path, "[diff]\ncontext = 17\n").unwrap();
        assert_eq!(
            load_from(&path).unwrap(),
            DiffOptions {
                context: 17,
                ..Default::default()
            }
        );
        for invalid in [
            "context = 10000001",
            "context = -1",
            "version = 'invalid'",
            "words = 'invalid'",
            "{broken",
        ] {
            fs::write(&path, format!("[diff]\n{invalid}")).unwrap();
            assert!(load_from(&path).is_err());
        }
        fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(load_from(&path).is_err());
    }

    #[test]
    fn worker_flushes_latest_options_on_exit_and_loads_them_next_time() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.toml");
        let engine = DiffEngine::with_options_file(Some(path.clone()));
        assert_eq!(engine.handle().initial_options(), DiffOptions::default());
        let mut options = DiffOptions::default();
        for context in 0..50 {
            options.context = context;
            engine.handle().save_options(options);
        }
        drop(engine);
        let next = DiffEngine::with_options_file(Some(path.clone()));
        assert_eq!(next.handle().initial_options(), options);
        assert!(!root.path().join("preferences.toml").exists());
    }

    #[test]
    fn read_only_file_survives_worker_save_failure() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.toml");
        let original = DiffOptions {
            context: 9,
            ..Default::default()
        };
        save_to(original, &path).unwrap();
        let permissions = fs::metadata(&path).unwrap().permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        let changed = DiffOptions {
            context: 20,
            ..original
        };
        assert!(save_to(changed, &path).is_err());
        let engine = DiffEngine::with_options_file(Some(path.clone()));
        engine.handle().save_options(changed);
        drop(engine);
        assert_eq!(load_from(&path).unwrap(), original);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[test]
    fn inaccessible_destination_and_invalid_startup_do_not_stop_worker() {
        let root = tempfile::tempdir().unwrap();
        let blocked = root.path().join("file");
        fs::write(&blocked, "not a directory").unwrap();
        let engine = DiffEngine::with_options_file(Some(blocked.join("state.toml")));
        assert_eq!(engine.handle().initial_options(), DiffOptions::default());
        engine.handle().save_options(DiffOptions {
            context: 7,
            ..Default::default()
        });
        drop(engine);
        assert_eq!(fs::read_to_string(blocked).unwrap(), "not a directory");
    }
}
