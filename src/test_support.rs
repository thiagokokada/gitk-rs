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

    /// Extend HEAD without a process per commit. Does not update the worktree.
    pub fn append_history(&self, count: usize, paths: bool, message: &str) -> String {
        use std::{io::Write, process::Stdio};
        let parent = self.git(&["rev-parse", "HEAD"]);
        let reference = self.git(&["symbolic-ref", "HEAD"]);
        let mut child = Command::new("git")
            .arg("-C")
            .arg(self.root())
            .args(["fast-import", "--quiet"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        {
            let mut input = child.stdin.take().unwrap();
            for i in 1..=count {
                writeln!(input, "commit {reference}\nmark :{i}\ncommitter Fixture <fixture@example.test> {} +0000\ndata {}", 1800000000 + i, message.len()).unwrap();
                input.write_all(message.as_bytes()).unwrap();
                writeln!(input).unwrap();
                if i == 1 {
                    writeln!(input, "from {parent}").unwrap();
                } else {
                    writeln!(input, "from :{}", i - 1).unwrap();
                }
                if paths {
                    let content = i.to_string();
                    writeln!(
                        input,
                        "M 100644 inline ci.yml\ndata {}\n{content}",
                        content.len()
                    )
                    .unwrap();
                }
                writeln!(input).unwrap();
            }
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.git(&["rev-parse", "HEAD"])
    }
}
