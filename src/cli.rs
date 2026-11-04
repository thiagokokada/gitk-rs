use std::{ffi::OsString, path::PathBuf};

pub const HELP: &str = "Usage: gitk-rs [--mode dark|light] [--timings] [--] [REPOSITORY]\n\
    --help, -h       Show help\n\
    --version, -V    Show version\n\
    --mode MODE     Choose dark (default) or light appearance\n\n\
    --timings       Report startup paint callbacks and bounded CPU frame samples\n\n\
    --limit N       Maximum commits per background batch (default 1000)\n\
    --graph-cols N  Visible graph lane cap, 1..200 (default 200)\n\
    --text-graph    Use a text graph instead of canvas geometry";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        }
    }
}

fn positive(args: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<usize, String> {
    args.next()
        .and_then(|arg| arg.to_str().and_then(|s| s.parse::<usize>().ok()))
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{flag} requires a positive integer"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Open(Config),
    Help,
    Version,
}

/// Parse OS strings so repository paths need not be valid UTF-8.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut repository = None;
    let mut theme = Theme::Dark;
    let mut positional_only = false;
    let mut timings = false;
    let mut batch = 1000;
    let mut graph_columns = 200;
    let mut text_graph = false;
    while let Some(arg) = args.next() {
        if !positional_only {
            match arg.to_str() {
                Some("--help" | "-h") => return Ok(Command::Help),
                Some("--version" | "-V") => return Ok(Command::Version),
                Some("--timings") => {
                    timings = true;
                    continue;
                }
                Some("--") => {
                    positional_only = true;
                    continue;
                }
                Some("--limit") => {
                    batch = positive(&mut args, "--limit")?;
                    continue;
                }
                Some("--graph-cols") => {
                    graph_columns = positive(&mut args, "--graph-cols")?;
                    if graph_columns > 200 {
                        return Err("--graph-cols must be between 1 and 200".into());
                    }
                    continue;
                }
                Some("--text-graph") => {
                    text_graph = true;
                    continue;
                }
                Some("--mode") => {
                    theme = match args.next().as_deref().and_then(|s| s.to_str()) {
                        Some("dark") => Theme::Dark,
                        Some("light") => Theme::Light,
                        _ => return Err("--mode requires dark or light".into()),
                    };
                    continue;
                }
                Some(value) if value.starts_with('-') => {
                    return Err(format!("unknown option: {value}"));
                }
                _ => {}
            }
        }
        if repository.replace(PathBuf::from(arg)).is_some() {
            return Err("expected at most one repository path".into());
        }
    }
    Ok(Command::Open(Config {
        repository: repository.unwrap_or_else(|| PathBuf::from(".")),
        theme,
        timings,
        batch,
        graph_columns,
        text_graph,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(OsString::from))
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
    fn rejects_invalid_options_and_ambiguous_paths() {
        for args in [
            &["--mode"][..],
            &["--mode", "blue"],
            &["--unknown"],
            &["a", "b"],
        ] {
            assert!(command(args).is_err());
        }
        assert_eq!(command(&["--help"]), Ok(Command::Help));
        assert_eq!(command(&["--version"]), Ok(Command::Version));
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
            parse([path.clone()]),
            Ok(Command::Open(Config {
                repository: path.into(),
                theme: Theme::Dark,
                timings: false,
                ..Config::default()
            }))
        );
    }
}
