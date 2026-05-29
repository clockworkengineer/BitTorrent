use crate::disk_io::DiskIO;
use crate::error::BitTorrentError;
use crate::metainfo::MetaInfoFile;
use crate::selector::Selector;
use crate::torrent_context::{TorrentContext, TorrentStatus};
use std::fs;
use std::path::{Path, PathBuf};

pub struct TorrentSession {
    pub context: TorrentContext,
    pub disk_io: DiskIO,
    pub download_path: PathBuf,
}

impl TorrentSession {
    pub fn new(
        torrent_path: impl AsRef<Path>,
        download_path: impl AsRef<Path>,
        seeding: bool,
    ) -> Result<Self, BitTorrentError> {
        let torrent_path = torrent_path.as_ref();
        let download_path = download_path.as_ref().to_path_buf();
        fs::create_dir_all(&download_path)?;

        let mut meta_info = MetaInfoFile::new(torrent_path)?;
        meta_info.parse()?;
        meta_info.validate()?;

        let disk_io = DiskIO::new();
        let selector = Selector::new();
        let context = TorrentContext::new(&meta_info, selector, &disk_io, &download_path, seeding)?;

        let session = TorrentSession {
            context,
            disk_io,
            download_path,
        };

        session.context.validate()?;
        Ok(session)
    }

    pub fn start_download(&mut self) -> Result<(), BitTorrentError> {
        if self.context.status == TorrentStatus::Seeding {
            return Err(BitTorrentError::Parse(
                "Cannot start download while torrent is configured for seeding.".into(),
            ));
        }
        self.context.start_downloading()
    }

    pub fn pause(&mut self) -> Result<(), BitTorrentError> {
        self.context.pause()
    }

    pub fn resume(&mut self) -> Result<(), BitTorrentError> {
        self.context.resume()
    }

    pub fn stop(&mut self) -> Result<(), BitTorrentError> {
        self.context.stop()
    }

    pub fn status(&self) -> TorrentStatus {
        self.context.status
    }

    pub fn progress(&self) -> f32 {
        self.context.progress_percent()
    }

    pub fn validate(&self) -> Result<(), BitTorrentError> {
        self.context.validate()?;
        for file in &self.context.files_to_download {
            let path = Path::new(&file.name);
            if !path.exists() {
                return Err(BitTorrentError::Parse(format!(
                    "Expected torrent file path is missing: {}",
                    file.name
                )));
            }
            let metadata = fs::metadata(path)?;
            if metadata.len() != file.length {
                return Err(BitTorrentError::Parse(format!(
                    "File length mismatch for {}: expected {} bytes, found {} bytes",
                    file.name,
                    file.length,
                    metadata.len()
                )));
            }
        }
        Ok(())
    }

    pub fn download_path(&self) -> &Path {
        &self.download_path
    }
}
