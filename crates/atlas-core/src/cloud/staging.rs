//! Sibling staging and exclusive publication: partial downloads are never the
//! final file, and a destination created during transfer is never overwritten.
use super::control::OutcomeUnknown;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub struct DownloadStage {
    root: Option<tempfile::TempDir>,
    pub path: PathBuf,
    destination: PathBuf,
    byte_limit: Option<u64>,
}

#[allow(dead_code)]
impl DownloadStage {
    pub fn new(destination: &Path) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !destination.try_exists()?,
            "Destination already exists: {}",
            destination.display()
        );
        let parent = destination
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Download destination has no parent"))?;
        let name = destination
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Download destination has no file name"))?;
        let root = tempfile::Builder::new()
            .prefix(".atlas-download-")
            .tempdir_in(parent)?;
        let path = root.path().join(name);
        Ok(Self {
            root: Some(root),
            path,
            destination: destination.to_path_buf(),
            byte_limit: None,
        })
    }

    pub fn set_byte_limit(&mut self, limit: Option<u64>) {
        self.byte_limit = limit;
    }

    pub fn check(&mut self, result: anyhow::Result<()>) -> anyhow::Result<()> {
        if result
            .as_ref()
            .is_err_and(|error| error.is::<OutcomeUnknown>())
        {
            // A disconnected backend may still be writing. Keep its private
            // staging directory for reconciliation; never expose it as complete.
            if let Some(root) = self.root.take() {
                let _ = root.keep();
            }
        }
        result
    }

    pub fn commit(self) -> anyhow::Result<()> {
        if let Some(limit) = self.byte_limit {
            let metadata = std::fs::metadata(&self.path)?;
            anyhow::ensure!(
                metadata.is_file() && metadata.len() <= limit,
                "Download exceeded its byte limit; the partial result was not published"
            );
        }
        #[cfg(windows)]
        atlas_win_space::pdf::publish_pages(&self.path, &self.destination)?;
        #[cfg(unix)]
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            &self.path,
            rustix::fs::CWD,
            &self.destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commit_never_replaces_a_destination_created_during_the_download() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("file.txt");
        let stage = DownloadStage::new(&destination).unwrap();
        std::fs::write(&stage.path, b"download").unwrap();
        assert!(!destination.exists());
        std::fs::write(&destination, b"created concurrently").unwrap();
        assert!(stage.commit().is_err());
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"created concurrently"
        );
    }
}
