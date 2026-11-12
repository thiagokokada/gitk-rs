use crate::{
    git::{DEFAULT_GIT_WORKERS, MAX_GIT_WORKERS},
    graph::{DEFAULT_GRAPH_LANES, MAX_GRAPH_LANES},
};
use clap::{CommandFactory, FromArgMatches, Parser, ValueEnum, parser::ValueSource};
use std::{ffi::OsString, num::NonZeroUsize, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub repository: PathBuf,
    pub theme: Theme,
    pub timings: bool,
    pub batch: Option<usize>,
    pub graph_columns: usize,
    pub git_workers: usize,
    pub text_graph: bool,
    pub syntax: bool,
    pub watch: bool,
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repository: PathBuf::from("."),
            theme: Theme::Dark,
            timings: false,
            batch: None,
            graph_columns: DEFAULT_GRAPH_LANES,
            git_workers: DEFAULT_GIT_WORKERS,
            text_graph: false,
            syntax: true,
            watch: true,
            verbose: false,
        }
    }
}

impl Config {
    /// Apply a complete set of defaults, including explicit edits made in Preferences.
    pub fn apply_preferences(&mut self, preferences: &crate::preferences::Preferences) {
        self.theme = preferences.theme;
        self.batch = preferences.batch;
        self.graph_columns = preferences.graph_columns;
        self.git_workers = preferences.git_workers;
        self.text_graph = preferences.text_graph;
        self.syntax = preferences.syntax;
        self.watch = preferences.watch;
    }
}

#[derive(Debug, Parser)]
#[command(name = "gitk-rs", version = crate::VERSION, about, after_help = "Saved preferences override the built-in defaults shown here. Explicit options override saved preferences. Boolean options accept =false.")]
struct Args {
    /// Git repository to browse
    #[arg(default_value = ".")]
    repository: PathBuf,

    /// Choose dark or light appearance
    #[arg(long, value_enum, default_value = "dark")]
    mode: Theme,

    /// Report startup paint callbacks and bounded CPU frame samples
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value = "false", require_equals = true)]
    timings: bool,

    /// Opt into batches of N instead of streaming all history (browsing loads more)
    #[arg(long, value_name = "N", conflicts_with = "no_limit")]
    limit: Option<NonZeroUsize>,

    /// Stream all history, overriding a saved prefetch batch size
    #[arg(long)]
    no_limit: bool,

    /// Logical graph lane cap before idle columns are folded (1..1024)
    #[arg(long, value_name = "N", default_value_t = DEFAULT_GRAPH_LANES as u16, value_parser = clap::value_parser!(u16).range(1..=MAX_GRAPH_LANES as i64))]
    graph_cols: u16,

    /// Git workers per history/content search pool (1..8)
    #[arg(long, value_name = "N", default_value_t = DEFAULT_GIT_WORKERS as u16, value_parser = clap::value_parser!(u16).range(1..=MAX_GIT_WORKERS as i64))]
    git_workers: u16,

    /// Use a text graph instead of canvas geometry
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value = "false", require_equals = true)]
    text_graph: bool,

    /// Disable syntax highlighting
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value = "false", require_equals = true)]
    nosyntax: bool,

    /// Disable automatic repository refresh
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value = "false", require_equals = true)]
    nowatch: bool,

    /// Report repository refreshes and operations
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value = "false", require_equals = true)]
    verbose: bool,
}

/// Parse arguments without argv[0], preserving non-UTF8 repository paths.
/// Clap returns help and version output as successful-exit errors.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Config, clap::Error> {
    parse_with_preferences(args, &crate::preferences::Preferences::default())
}

pub fn parse_with_preferences(
    args: impl IntoIterator<Item = OsString>,
    preferences: &crate::preferences::Preferences,
) -> Result<Config, clap::Error> {
    let matches = Args::command()
        .try_get_matches_from(std::iter::once(OsString::from("gitk-rs")).chain(args))?;
    let args = Args::from_arg_matches(&matches)?;
    let explicit = |name| matches.value_source(name) == Some(ValueSource::CommandLine);
    let mut config = Config {
        repository: args.repository,
        ..Config::default()
    };
    config.apply_preferences(preferences);
    if explicit("mode") {
        config.theme = args.mode;
    }
    if explicit("timings") {
        config.timings = args.timings;
    }
    if explicit("limit") || explicit("no_limit") {
        config.batch = args.limit.map(NonZeroUsize::get);
    }
    if explicit("graph_cols") {
        config.graph_columns = usize::from(args.graph_cols);
    }
    if explicit("git_workers") {
        config.git_workers = usize::from(args.git_workers);
    }
    if explicit("text_graph") {
        config.text_graph = args.text_graph;
    }
    if explicit("nosyntax") {
        config.syntax = !args.nosyntax;
    }
    if explicit("nowatch") {
        config.watch = !args.nowatch;
    }
    if explicit("verbose") {
        config.verbose = args.verbose;
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_definition_is_consistent() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }

    fn command(args: &[&str]) -> Result<Config, String> {
        parse(args.iter().map(OsString::from)).map_err(|error| error.to_string())
    }

    #[test]
    fn defaults_and_explicit_theme() {
        assert_eq!(
            command(&[]),
            Ok(Config {
                repository: PathBuf::from("."),
                theme: Theme::Dark,
                timings: false,
                ..Config::default()
            })
        );
        assert_eq!(
            command(&["--mode", "light", "repo with spaces"]),
            Ok(Config {
                repository: PathBuf::from("repo with spaces"),
                theme: Theme::Light,
                timings: false,
                ..Config::default()
            })
        );
    }

    #[test]
    fn syntax_can_be_disabled() {
        let config = command(&["--nosyntax"]).unwrap();
        assert!(!config.syntax);
    }

    #[test]
    fn explicit_flags_override_saved_settings_including_default_and_false_values() {
        let preferences = crate::preferences::Preferences {
            theme: Theme::Light,
            batch: Some(73),
            graph_columns: 1024,
            git_workers: 8,
            text_graph: true,
            syntax: false,
            watch: false,
            ..crate::preferences::Preferences::default()
        };
        let args = [
            "--mode=dark",
            "--timings=false",
            "--no-limit",
            "--graph-cols=256",
            "--git-workers=4",
            "--text-graph=false",
            "--nosyntax=false",
            "--nowatch=false",
            "--verbose=false",
        ];
        assert_eq!(
            parse_with_preferences(args.map(OsString::from), &preferences).unwrap(),
            Config::default()
        );
        let partial = parse_with_preferences(["--mode=dark".into()], &preferences).unwrap();
        assert_eq!(partial.theme, Theme::Dark);
        assert_eq!(partial.batch, Some(73));
        assert!(!partial.syntax && !partial.watch);
        assert!(!partial.timings && !partial.verbose);
        assert_eq!(partial.graph_columns, 1024);
        assert_eq!(partial.git_workers, 8);
        let mut diagnostics = parse(["--timings".into(), "--verbose".into()]).unwrap();
        diagnostics.apply_preferences(&preferences);
        assert!(diagnostics.timings && diagnostics.verbose);
        assert!(parse(["--limit=5".into(), "--no-limit".into()]).is_err());
    }

    #[test]
    fn parses_all_viewer_options_and_equals_syntax() {
        assert_eq!(
            command(&[
                "repo with spaces",
                "--mode=light",
                "--limit=42",
                "--graph-cols=1",
                "--text-graph",
                "--nosyntax",
                "--nowatch",
                "--timings",
                "--verbose",
            ]),
            Ok(Config {
                repository: "repo with spaces".into(),
                theme: Theme::Light,
                timings: true,
                batch: Some(42),
                graph_columns: 1,
                git_workers: DEFAULT_GIT_WORKERS,
                text_graph: true,
                syntax: false,
                watch: false,
                verbose: true,
            })
        );
    }

    #[test]
    fn validates_numeric_limits() {
        for args in [
            &["--limit"][..],
            &["--limit", "0"],
            &["--limit", "-1"],
            &["--limit", "not-a-number"],
            &["--limit", "999999999999999999999999999999"],
            &["--graph-cols"],
            &["--graph-cols", "0"],
            &["--graph-cols", "1025"],
            &["--graph-cols", "65536"],
            &["--git-workers"],
            &["--git-workers", "0"],
            &["--git-workers", "9"],
            &["--git-workers", "invalid"],
            &["--git-workers", "-1"],
        ] {
            assert!(
                command(args).is_err(),
                "accepted invalid arguments: {args:?}"
            );
        }
        for value in ["1", "256", "512", "1024"] {
            assert_eq!(
                command(&["--graph-cols", value]).unwrap().graph_columns,
                value.parse::<usize>().unwrap()
            );
        }
        assert_eq!(command(&[]).unwrap().graph_columns, 256);
        assert_eq!(command(&[]).unwrap().git_workers, 4);
        for readers in [1, 2, 4, 8] {
            assert_eq!(
                command(&["--git-workers", &readers.to_string()])
                    .unwrap()
                    .git_workers,
                readers
            );
        }
    }

    #[test]
    fn rejects_invalid_options_and_ambiguous_paths() {
        for args in [
            &["--mode"][..],
            &["--mode", "blue"],
            &["--unknown"],
            &["a", "b"],
        ] {
            assert!(command(args).is_err());
        }
        assert_eq!(
            parse(["--help".into()]).unwrap_err().kind(),
            clap::error::ErrorKind::DisplayHelp
        );
        assert_eq!(
            parse(["--version".into()]).unwrap_err().kind(),
            clap::error::ErrorKind::DisplayVersion
        );
    }

    #[test]
    fn option_terminator_preserves_dash_path() {
        assert_eq!(
            command(&["--", "--help"]),
            Ok(Config {
                repository: PathBuf::from("--help"),
                theme: Theme::Dark,
                timings: false,
                ..Config::default()
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_repository_paths() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"repo-\xff".to_vec());
        assert_eq!(
            parse([path.clone()]).unwrap(),
            Config {
                repository: path.into(),
                theme: Theme::Dark,
                timings: false,
                ..Config::default()
            }
        );
    }
}
