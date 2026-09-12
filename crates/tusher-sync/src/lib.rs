// crates/tusher-sync/src/lib.rs
// Native filesystem watcher and automated two-way synchronization engine for T.U.S.H.E.R.

pub mod watcher;
pub mod coordinator;

pub use coordinator::SyncCoordinator;
pub use watcher::{FolderWatcher, FsChangeEvent};
