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
        assert!(help.contains("--list-languages"));
        assert!(help.contains("--nowatch"));
    }
    for flag in ["--version", "-V"] {
        let output = run(&[flag]);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            concat!("gitk-rs ", env!("CARGO_PKG_VERSION"), "\n")
        );
    }
}

#[test]
fn invalid_arguments_exit_with_diagnostics_on_stderr() {
    for args in [
        &["--unknown"][..],
        &["--mode", "blue"],
        &["--limit", "0"],
        &["--graph-cols", "201"],
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

#[test]
fn language_inventory_exits_without_opening_the_gui() {
    let output = run(&["--list-languages"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8(output.stdout).unwrap().contains("Rust"));
}
