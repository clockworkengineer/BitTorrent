use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher, EventKind};

#[allow(dead_code)]
#[derive(Debug)]
pub enum FsEvent {
    Updated(PathBuf),
    Deleted(PathBuf),
}

pub struct DirectoryWatcher {
    _watcher: RecommendedWatcher,
    #[allow(dead_code)]
    rx: Receiver<FsEvent>,
}

impl DirectoryWatcher {
    pub fn watch(path: impl AsRef<Path>) -> Result<Self, notify::Error> {
        let (tx, rx) = channel();
        let path_to_watch = path.as_ref().to_path_buf();
        let path_to_watch_clone = path_to_watch.clone();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) => {
                            for p in event.paths {
                                if p.exists() {
                                    // Make path relative to watched root if possible
                                    if let Ok(rel) = p.strip_prefix(&path_to_watch_clone) {
                                        let _ = tx.send(FsEvent::Updated(rel.to_path_buf()));
                                    }
                                }
                            }
                        }
                        EventKind::Remove(_) => {
                            for p in event.paths {
                                if let Ok(rel) = p.strip_prefix(&path_to_watch_clone) {
                                    let _ = tx.send(FsEvent::Deleted(rel.to_path_buf()));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            },
            Config::default().with_poll_interval(Duration::from_millis(500)),
        )?;

        watcher.watch(&path_to_watch, RecursiveMode::Recursive)?;

        Ok(DirectoryWatcher {
            _watcher: watcher,
            rx,
        })
    }

    #[allow(dead_code)]
    pub fn receiver(&self) -> &Receiver<FsEvent> {
        &self.rx
    }
}
