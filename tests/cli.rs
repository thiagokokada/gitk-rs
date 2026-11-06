//! Exercise CLI exits through the executable without a display or repository.
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gitk-rs"))
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("run gitk-rs")
}

#[test]
fn help_and_version_exit_successfully_on_stdout() {
    for flag in ["--help", "-h"] {
        let output = run(&[flag]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("Usage: gitk-rs"));
        assert!(help.contains("--graph-cols"));
        assert!(help.contains("1..1024"));
        assert!(help.contains("[default: 256]"));
        assert!(help.contains("--nowatch"));
    }
    let revision = gitk_rs::VERSION
        .strip_prefix(concat!(env!("CARGO_PKG_VERSION"), "-"))
        .expect("version includes the base package version");
    assert!(
        revision == "unknown"
            || (revision.len() == 7 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
    );
    for flag in ["--version", "-V"] {
        let output = run(&[flag]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("gitk-rs {}\n", gitk_rs::VERSION)
        );
    }
}

#[test]
fn invalid_arguments_exit_with_diagnostics_on_stderr() {
    for args in [
        &["--unknown"][..],
        &["--mode", "blue"],
        &["--limit", "0"],
        &["--graph-cols", "1025"],
        &["one", "two"],
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2), "arguments: {args:?}");
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("error:"));
        assert!(error.contains("--help"), "missing help guidance: {error}");
    }
}
