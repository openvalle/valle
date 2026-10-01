//! Stage an entire delivery before publishing any files, without replacing existing outputs.

use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(super) struct OutputTransaction {
    entries: Vec<(PathBuf, Option<tempfile::TempPath>)>,
    published: Vec<PathBuf>,
    committed: bool,
}

impl OutputTransaction {
    pub fn new(outputs: &[PathBuf]) -> Result<Self> {
        let mut unique = BTreeSet::new();
        let mut destinations = Vec::new();
        for output in outputs {
            let name = output.file_name().context("output must have a file name")?;
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
            let destination = parent.canonicalize()?.join(name);
            if !unique.insert(destination.clone()) {
                bail!("duplicate output {}", output.display());
            }
            match std::fs::symlink_metadata(&destination) {
                Ok(_) => bail!("refusing to overwrite existing output {}", output.display()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            destinations.push(destination);
        }
        let mut entries = Vec::new();
        for destination in destinations {
            let suffix = destination
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| format!(".{ext}"))
                .unwrap_or_default();
            let mut builder = tempfile::Builder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Match normal output creation, including the caller's umask.
                builder.permissions(std::fs::Permissions::from_mode(0o666));
            }
            let temporary = builder
                .prefix(".valle-part-")
                .suffix(&suffix)
                .tempfile_in(destination.parent().expect("absolute output parent"))?
                .into_temp_path();
            entries.push((destination, Some(temporary)));
        }
        Ok(Self {
            entries,
            published: Vec::new(),
            committed: false,
        })
    }

    pub fn temporary(&self, index: usize) -> &Path {
        self.entries[index]
            .1
            .as_deref()
            .expect("unpublished output")
    }

    pub fn publish(mut self) -> Result<()> {
        for (destination, temporary) in &mut self.entries {
            temporary
                .take()
                .expect("unpublished output")
                .persist_noclobber(&*destination)
                .with_context(|| {
                    format!("publish {} without overwriting", destination.display())
                })?;
            self.published.push(destination.clone());
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for OutputTransaction {
    fn drop(&mut self) {
        if !self.committed {
            for output in &self.published {
                let _ = std::fs::remove_file(output);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_collision_rolls_back_our_outputs_and_preserves_the_other_writer() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.png");
        let second = dir.path().join("second.png");
        let transaction = OutputTransaction::new(&[first.clone(), second.clone()]).unwrap();
        std::fs::write(transaction.temporary(0), b"first").unwrap();
        std::fs::write(transaction.temporary(1), b"second").unwrap();
        std::fs::write(&second, b"other writer").unwrap();
        assert!(transaction.publish().is_err());
        assert!(!first.exists());
        assert_eq!(std::fs::read(&second).unwrap(), b"other writer");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn aborted_delivery_removes_all_staging_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = [dir.path().join("a.png"), dir.path().join("b.png")];
        let transaction = OutputTransaction::new(&paths).unwrap();
        std::fs::write(transaction.temporary(0), b"partial frame").unwrap();
        drop(transaction);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[cfg(all(test, unix))]
mod permission_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn published_outputs_have_normal_creation_permissions_and_honor_umask() {
        let directory = tempfile::tempdir().unwrap();
        let ordinary = directory.path().join("ordinary");
        std::fs::write(&ordinary, b"normal creation").unwrap();
        let expected = std::fs::metadata(ordinary).unwrap().permissions().mode() & 0o777;
        let paths = ["frame.png", "sequence-0.png", "sheet.png", "alpha.mov"]
            .map(|name| directory.path().join(name));
        let transaction = OutputTransaction::new(&paths).unwrap();
        for index in 0..paths.len() {
            std::fs::write(transaction.temporary(index), b"output").unwrap();
        }
        transaction.publish().unwrap();
        for path in paths {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                expected
            );
        }
    }
}
