//! Atomic settings writes that respect an existing file's write permissions.
use anyhow::{Context, Result, ensure};
use std::{fs, io::Write, path::Path};

pub(crate) fn write(path: &Path, contents: &[u8]) -> Result<()> {
    (|| {
        // Rename alone can replace a read-only file in a writable directory.
        // Verify actual write access without truncation before creating a replacement.
        let permissions = match fs::metadata(path) {
            Ok(metadata) => {
                ensure!(
                    !metadata.permissions().readonly(),
                    "settings file is read-only"
                );
                fs::OpenOptions::new().write(true).open(path)?;
                Some(metadata.permissions())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let parent = path.parent().context("settings directory is missing")?;
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(contents)?;
        if let Some(permissions) = permissions {
            temporary.as_file().set_permissions(permissions)?;
        }
        temporary.as_file().sync_all()?;
        temporary.persist(path)?;
        Ok(())
    })()
    .with_context(|| format!("write settings {}", path.display()))
}
