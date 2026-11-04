use clap::{Parser, ValueEnum};
use std::{ffi::OsString, num::NonZeroUsize, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub repository: PathBuf,
    pub theme: Theme,
    pub timings: bool,
    pub batch: usize,
    pub graph_columns: usize,
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
            batch: 1000,
            graph_columns: 200,
            text_graph: false,
            syntax: true,
            watch: true,
            verbose: false,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "gitk-rs", version, about)]
struct Args {
    /// Git repository to browse
    #[arg(default_value = ".")]
    repository: PathBuf,

    /// Choose dark or light appearance
    #[arg(long, value_enum, default_value = "dark")]
    mode: Theme,

    /// Report startup paint callbacks and bounded CPU frame samples
    #[arg(long)]
    timings: bool,

    /// Maximum commits per background batch
    #[arg(long, value_name = "N", default_value = "1000")]
    limit: NonZeroUsize,

    /// Visible graph lane cap (1..200)
    #[arg(long, value_name = "N", default_value = "200", value_parser = clap::value_parser!(u16).range(1..=200))]
    graph_cols: u16,

    /// Use a text graph instead of canvas geometry
    #[arg(long)]
    text_graph: bool,

    /// Disable syntax highlighting
    #[arg(long)]
    nosyntax: bool,

    /// List bundled languages and extensions
    #[arg(long)]
    list_languages: bool,

    /// Disable automatic repository refresh
    #[arg(long)]
    nowatch: bool,

    /// Report repository refreshes and operations
    #[arg(long)]
    verbose: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Open(Config),
    Languages,
}

/// Parse arguments without argv[0], preserving non-UTF8 repository paths.
/// Clap returns help and version output as successful-exit errors.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, clap::Error> {
    let args = Args::try_parse_from(std::iter::once(OsString::from("gitk-rs")).chain(args))?;
    if args.list_languages {
        return Ok(Command::Languages);
    }
    Ok(Command::Open(Config {
        repository: args.repository,
        theme: args.mode,
        timings: args.timings,
        batch: args.limit.get(),
        graph_columns: usize::from(args.graph_cols),
        text_graph: args.text_graph,
        syntax: !args.nosyntax,
        watch: !args.nowatch,
        verbose: args.verbose,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_definition_is_consistent() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }

    fn command(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(OsString::from)).map_err(|error| error.to_string())
    }

    #[test]
    fn defaults_and_explicit_theme() {
        assert_eq!(
            command(&[]),
            Ok(Command::Open(Config {
                repository: PathBuf::from("."),
                theme: Theme::Dark,
                timings: false,
                ..Config::default()
            }))
        );
        assert_eq!(
            command(&["--mode", "light", "repo with spaces"]),
            Ok(Command::Open(Config {
                repository: PathBuf::from("repo with spaces"),
                theme: Theme::Light,
                timings: false,
                ..Config::default()
            }))
        );
    }

    #[test]
    fn syntax_controls_and_inventory_commands() {
        assert_eq!(command(&["--list-languages"]), Ok(Command::Languages));
        let Command::Open(config) = command(&["--nosyntax"]).unwrap() else {
            panic!("expected config");
        };
        assert!(!config.syntax);
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
            Ok(Command::Open(Config {
                repository: "repo with spaces".into(),
                theme: Theme::Light,
                timings: true,
                batch: 42,
                graph_columns: 1,
                text_graph: true,
                syntax: false,
                watch: false,
                verbose: true,
            }))
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
            &["--graph-cols", "201"],
            &["--graph-cols", "65536"],
        ] {
            assert!(
                command(args).is_err(),
                "accepted invalid arguments: {args:?}"
            );
        }
        assert!(command(&["--limit", "1", "--graph-cols", "200"]).is_ok());
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
            Ok(Command::Open(Config {
                repository: PathBuf::from("--help"),
                theme: Theme::Dark,
                timings: false,
                ..Config::default()
            }))
        );
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_repository_paths() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"repo-\xff".to_vec());
        assert_eq!(
            parse([path.clone()]).unwrap(),
            Command::Open(Config {
                repository: path.into(),
                theme: Theme::Dark,
                timings: false,
                ..Config::default()
            })
        );
    }
}
