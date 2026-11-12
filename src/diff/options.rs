//! Automatically saved window state, independent of explicitly applied preferences.
use super::DiffOptions;
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct State {
    pub diff: DiffOptions,
    pub layout: Layout,
    pub search: Search,
}

impl State {
    fn validate(self) -> Result<()> {
        ensure!(
            self.diff.context <= 10_000_000,
            "diff context exceeds ten million lines"
        );
        self.layout.validate()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Search {
    pub regex: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Layout {
    pub graph_width: f32,
    pub history_height: f32,
    pub file_list_width: f32,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            graph_width: 280.,
            history_height: 265.,
            file_list_width: 280.,
        }
    }
}
#[derive(Clone, Copy)]
pub enum LayoutChange {
    GraphWidth(f32),
    HistoryHeight(f32),
    FileListWidth(f32),
}
impl Layout {
    pub fn apply(&mut self, change: LayoutChange) {
        match change {
            LayoutChange::GraphWidth(value) => self.graph_width = value,
            LayoutChange::HistoryHeight(value) => self.history_height = value,
            LayoutChange::FileListWidth(value) => self.file_list_width = value,
        }
    }
    fn validate(self) -> Result<()> {
        ensure!(
            self.graph_width.is_finite() && (160. ..=720.).contains(&self.graph_width),
            "invalid graph width"
        );
        for value in [self.history_height, self.file_list_width] {
            ensure!(
                value.is_finite() && (80. ..=100_000.).contains(&value),
                "invalid pane size"
            );
        }
        Ok(())
    }
}

pub fn path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("cannot locate configuration directory")?
        .join("gitk-rs/state.toml"))
}

pub fn load_from(path: &Path) -> Result<DiffOptions> {
    Ok(load_state_from(path)?.diff)
}

pub fn load_state_from(path: &Path) -> Result<State> {
    let state: State = crate::settings_file::read_toml(path, "diff options exceed 64 KiB")?;
    state.validate()?;
    Ok(state)
}

pub fn save_to(options: DiffOptions, path: &Path) -> Result<()> {
    let mut state = load_state_from(path)?;
    state.diff = options;
    save_state_to(state, path)
}

pub fn save_state_to(state: State, path: &Path) -> Result<()> {
    state.validate()?;
    crate::settings_file::write(path, toml::to_string_pretty(&state)?.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffEngine, DiffVersion, WordDiff};
    use std::fs;

    #[test]
    fn invalid_state_is_rejected_before_replacing_saved_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.toml");
        let original = State::default();
        save_state_to(original, &path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let mut invalid_diff = original;
        invalid_diff.diff.context = 10_000_001;
        let mut invalid_layout = original;
        invalid_layout.layout.graph_width = 900.;
        for state in [invalid_diff, invalid_layout] {
            assert!(save_state_to(state, &path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(load_state_from(&path).unwrap(), original);
        }
    }

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

#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::diff::{DiffEngine, WordDiff};
    use std::fs;

    #[test]
    fn worker_flushes_latest_diff_layout_and_search_on_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.toml");
        // Existing diff-only state files remain readable.
        fs::write(&path, "[diff]\ncontext = 7\n").unwrap();
        assert_eq!(load_state_from(&path).unwrap().layout, Layout::default());
        assert!(!load_state_from(&path).unwrap().search.regex);
        let engine = DiffEngine::with_options_file(Some(path.clone()));
        let handle = engine.handle();
        handle.save_layout(LayoutChange::GraphWidth(410.));
        handle.save_layout(LayoutChange::HistoryHeight(330.));
        let mut diff = DiffOptions {
            words: WordDiff::Markup,
            ..handle.initial_options()
        };
        for context in 0..50 {
            diff.context = context;
            handle.save_options(diff);
        }
        handle.save_search(Search { regex: true });
        handle.save_layout(LayoutChange::FileListWidth(350.));
        drop(engine);
        let saved = load_state_from(&path).unwrap();
        assert_eq!(saved.diff, diff);
        assert!(saved.search.regex);
        let next = DiffEngine::with_options_file(Some(path.clone()));
        assert_eq!(next.handle().initial_options(), diff);
        assert!(next.handle().initial_search().regex);
        drop(next);
        assert_eq!(
            saved.layout,
            Layout {
                graph_width: 410.,
                history_height: 330.,
                file_list_width: 350.
            }
        );
        save_to(DiffOptions::default(), &path).unwrap();
        assert_eq!(load_state_from(&path).unwrap().layout, saved.layout);
        assert!(load_state_from(&path).unwrap().search.regex);
        assert!(!root.path().join("preferences.toml").exists());
    }

    #[test]
    fn invalid_layout_sizes_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.toml");
        for value in [
            "graph_width = -1",
            "graph_width = 900",
            "history_height = nan",
            "file_list_width = inf",
            "file_list_width = 0",
        ] {
            fs::write(&path, format!("[layout]\n{value}\n")).unwrap();
            assert!(load_state_from(&path).is_err());
        }
    }
}
