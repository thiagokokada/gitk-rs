use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

pub struct Fixture {
    pub directory: TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let fixture = Self {
            directory: tempfile::tempdir().unwrap(),
        };
        fixture.git(&["init", "--quiet", "--initial-branch=main"]);
        fixture.git(&["config", "core.hooksPath", "/dev/null"]);
        fixture.git(&["config", "core.fsmonitor", "false"]);
        fixture
    }

    pub fn root(&self) -> &Path {
        self.directory.path()
    }

    pub fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-C")
            .arg(self.root())
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned()
    }

    pub fn commit(&self, message: &str) -> String {
        fs::write(self.root().join("file.txt"), message).unwrap();
        self.git(&["add", "--", "file.txt"]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }
}
