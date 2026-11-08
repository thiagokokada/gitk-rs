//! Bounded TOML reads and atomic writes that respect existing file permissions.
use anyhow::{Context, Result, ensure};
use serde::de::DeserializeOwned;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

const MAX_BYTES: usize = 64 * 1024;

pub(crate) fn read_toml<T: DeserializeOwned + Default>(path: &Path, size_error: &str) -> Result<T> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_BYTES, "{size_error}");
    Ok(toml::from_str(std::str::from_utf8(&bytes)?)?)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_read_accepts_the_size_limit_and_rejects_one_more_byte() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.toml");
        let mut contents = b"value = 7\n#".to_vec();
        contents.resize(MAX_BYTES, b' ');
        fs::write(&path, &contents).unwrap();
        let parsed: toml::Table = read_toml(&path, "settings exceed 64 KiB").unwrap();
        assert_eq!(parsed["value"].as_integer(), Some(7));
        contents.push(b' ');
        fs::write(&path, contents).unwrap();
        let error = read_toml::<toml::Table>(&path, "settings exceed 64 KiB").unwrap_err();
        assert_eq!(error.to_string(), "settings exceed 64 KiB");
    }
}
