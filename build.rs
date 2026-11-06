use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn git(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GITK_RS_REVISION");
    let directory =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let revision = env::var("GITK_RS_REVISION").ok().or_else(|| {
        let revision = git(&directory, &["rev-parse", "--verify", "HEAD"])?;
        // Watch only revision metadata, including linked worktrees and packed refs.
        let reference = git(&directory, &["symbolic-ref", "--quiet", "HEAD"]);
        for name in [Some("HEAD"), Some("packed-refs"), reference.as_deref()]
            .into_iter()
            .flatten()
        {
            if let Some(path) = git(&directory, &["rev-parse", "--git-path", name]) {
                let path = directory.join(path);
                // A missing loose branch ref still needs tracking: a later commit
                // may create it without changing HEAD or the packed-refs file.
                if name != "packed-refs" || path.exists() {
                    println!("cargo:rerun-if-changed={}", path.display());
                }
            }
        }
        Some(revision)
    });
    let revision = match revision.as_deref() {
        None | Some("unknown") => "unknown".to_owned(),
        Some(revision) => {
            assert!(
                (7..=64).contains(&revision.len())
                    && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "GITK_RS_REVISION must be a Git commit ID (at least seven hex digits) or unknown"
            );
            revision[..7].to_ascii_lowercase()
        }
    };
    println!("cargo:rustc-env=GITK_RS_REVISION={revision}");
}
