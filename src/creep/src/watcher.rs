use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::RecursiveMode;
use notify_debouncer_mini::{Debouncer, new_debouncer};
use tokio::sync::{Mutex, mpsc};

use crate::index::FileIndex;
use crate::lsp::manager::LspManager;

/// Events produced by the file watcher.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

/// Watches one or more workspace directories for file changes, filtering out
/// gitignored paths using the `ignore` crate.
pub struct FileWatcher {
    /// One debouncer per workspace so we can stop watching individually.
    watchers: HashMap<PathBuf, Debouncer<notify::RecommendedWatcher>>,
    /// Gitignore matcher per workspace root, built when a workspace is registered.
    matchers: HashMap<PathBuf, ignore::gitignore::Gitignore>,
    /// Sender half of the event channel — cloned into each debouncer closure.
    tx: mpsc::Sender<WatchEvent>,
}

impl FileWatcher {
    /// Create a new `FileWatcher` associated with `index`.
    ///
    /// Returns:
    /// - An `Arc<Mutex<FileWatcher>>` so the gRPC service can call
    ///   `watch`/`unwatch` and the event processor can call `is_ignored`.
    /// - An `mpsc::Receiver<WatchEvent>` — feed this to [`process_events`].
    pub fn new(_index: FileIndex) -> (Arc<Mutex<Self>>, mpsc::Receiver<WatchEvent>) {
        let (tx, rx) = mpsc::channel::<WatchEvent>(1024);
        let watcher = Arc::new(Mutex::new(Self {
            watchers: HashMap::new(),
            matchers: HashMap::new(),
            tx,
        }));
        (watcher, rx)
    }

    /// Start watching `workspace` recursively.
    ///
    /// Builds a gitignore matcher for the workspace and registers a notify
    /// watcher whose callback forwards events onto the shared channel.
    pub fn watch(&mut self, workspace: &Path) -> anyhow::Result<()> {
        let workspace_buf = workspace.to_path_buf();

        // Build gitignore matcher from .gitignore in the workspace root.
        let mut builder = ignore::gitignore::GitignoreBuilder::new(workspace);
        let gitignore_path = workspace.join(".gitignore");
        if gitignore_path.exists() {
            builder.add(&gitignore_path);
        }
        let matcher = match builder.build() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    workspace = %workspace.display(),
                    error = %e,
                    "failed to parse .gitignore, nothing will be ignored in this workspace"
                );
                ignore::gitignore::Gitignore::empty()
            }
        };
        self.matchers.insert(workspace_buf.clone(), matcher);

        // Create a debounced watcher. The closure runs on the debouncer's
        // background thread so we use `try_send` (non-blocking).
        let tx = self.tx.clone();
        let mut debouncer = new_debouncer(
            Duration::from_millis(500),
            move |res: notify_debouncer_mini::DebounceEventResult| {
                let events = match res {
                    Ok(events) => events,
                    Err(err) => {
                        tracing::error!("creep: notify watcher error: {err}");
                        return;
                    }
                };
                for event in events {
                    // known race condition and limit of debounce
                    let watch_event = if event.path.exists() {
                        WatchEvent::Modified(event.path)
                    } else {
                        WatchEvent::Removed(event.path)
                    };
                    if let Err(e) = tx.try_send(watch_event) {
                        // Rate-limit overflow warnings to avoid log spam.
                        static LAST_WARN: std::sync::Mutex<Option<Instant>> =
                            std::sync::Mutex::new(None);
                        let mut last = LAST_WARN.lock().unwrap_or_else(|e| e.into_inner());
                        if last.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
                            tracing::warn!("creep: watcher event dropped: {e}");
                            *last = Some(Instant::now());
                        }
                    }
                }
            },
        )?;

        debouncer
            .watcher()
            .watch(workspace, RecursiveMode::Recursive)?;
        self.watchers.insert(workspace_buf, debouncer);
        Ok(())
    }

    /// Stop watching `workspace`.  Drops the notify watcher, which stops the OS watch.
    pub fn unwatch(&mut self, workspace: &Path) {
        self.watchers.remove(workspace);
        self.matchers.remove(workspace);
    }

    /// Returns `true` if `path` is ignored according to any loaded gitignore
    /// matcher whose root is a prefix of `path`.
    pub fn is_ignored(&self, path: &Path) -> bool {
        let is_dir = path.is_dir();
        for (root, matcher) in &self.matchers {
            if path.starts_with(root) && matcher.matched(path, is_dir).is_ignore() {
                return true;
            }
        }
        false
    }
}

/// Async task that reads `WatchEvent`s from `event_rx` and updates `index`,
/// skipping paths that `watcher.is_ignored()` reports as gitignored.
/// Also forwards file events to the LSP manager as document notifications.
///
/// Runs until the sender side of `event_rx` is dropped (all watcher instances gone).
pub async fn process_events(
    index: FileIndex,
    symbol_index: crate::symbol_index::SymbolIndex,
    watcher: Arc<Mutex<FileWatcher>>,
    lsp_manager: Arc<Mutex<LspManager>>,
    mut event_rx: mpsc::Receiver<WatchEvent>,
) {
    while let Some(event) = event_rx.recv().await {
        let is_create = matches!(&event, WatchEvent::Created(_));
        let path = match &event {
            WatchEvent::Created(p) | WatchEvent::Modified(p) | WatchEvent::Removed(p) => p.clone(),
        };

        // Check gitignore before touching the index.
        {
            let guard = watcher.lock().await;
            if guard.is_ignored(&path) {
                tracing::trace!("ignoring event for {}", path.display());
                continue;
            }
        }

        match event {
            WatchEvent::Created(p) | WatchEvent::Modified(p) => {
                if p.is_file() {
                    if let Err(e) = index.update_file(&p).await {
                        tracing::warn!("failed to index {}: {}", p.display(), e);
                    } else {
                        tracing::debug!("indexed {}", p.display());
                    }
                    let si = symbol_index.clone();
                    let p2 = p.clone();
                    match tokio::task::spawn_blocking(move || si.reparse_file(&p2)).await {
                        Ok(Err(e)) => {
                            tracing::warn!("symbol reparse failed for {}: {e}", p.display())
                        }
                        Err(e) => {
                            tracing::warn!("symbol reparse task panicked for {}: {e}", p.display())
                        }
                        _ => {}
                    }

                    // Notify LSP servers about the file change.
                    notify_lsp_file_change(&lsp_manager, &p, is_create).await;
                }
            }
            WatchEvent::Removed(p) => {
                index.remove_file(&p).await;
                symbol_index.remove_file(&p).await;
                tracing::debug!("removed {} from index", p.display());

                // Notify LSP servers about the file closure.
                notify_lsp_file_close(&lsp_manager, &p).await;
            }
        }
    }
}

/// Send didOpen or didChange to the appropriate LSP server for a file.
async fn notify_lsp_file_change(
    lsp_manager: &Arc<Mutex<LspManager>>,
    path: &Path,
    is_create: bool,
) {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => format!(".{e}"),
        None => return,
    };

    let mut mgr = lsp_manager.lock().await;

    // Look up language_id before ensure_server (which borrows &mut self).
    let language_id = match mgr.language_id_for_extension(&ext) {
        Some(id) => id.to_string(),
        None => return,
    };

    // Find (or start) the server for this workspace + extension.
    let workspace = match mgr.find_workspace_for_file(path) {
        Some(ws) => ws,
        None => return,
    };

    if let Err(e) = mgr.ensure_server(&workspace, &ext).await {
        tracing::warn!("failed to start LSP server for {}: {e}", path.display());
        return;
    }

    let path_str = path.to_string_lossy();
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("cannot read {} for LSP notification: {e}", path.display());
            return;
        }
    };

    if let Some(client) = mgr.get_client(&workspace, &ext) {
        let result = if is_create {
            client
                .open_document(&path_str, &content, &language_id)
                .await
        } else {
            // Use version 0 for file-watcher driven changes — the LSP server
            // tracks versions per-URI independently.
            client.change_document(&path_str, &content, 0).await
        };
        if let Err(e) = result {
            tracing::warn!("LSP notification failed for {}: {e}", path.display());
        }
    }
}

/// Send didClose to the appropriate LSP server for a removed file.
async fn notify_lsp_file_close(lsp_manager: &Arc<Mutex<LspManager>>, path: &Path) {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => format!(".{e}"),
        None => return,
    };

    let mgr = lsp_manager.lock().await;
    let workspace = match mgr.find_workspace_for_file(path) {
        Some(ws) => ws,
        None => return,
    };

    if let Some(client) = mgr.get_client(&workspace, &ext) {
        let path_str = path.to_string_lossy();
        if let Err(e) = client.close_document(&path_str).await {
            tracing::warn!("LSP close notification failed for {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn empty_lsp_manager() -> Arc<Mutex<LspManager>> {
        Arc::new(Mutex::new(LspManager::new(vec![])))
    }

    #[tokio::test]
    async fn test_watch_and_unwatch() {
        let dir = tempfile::tempdir().unwrap();
        let index = FileIndex::new();
        let (fw, _rx) = FileWatcher::new(index);
        let mut guard = fw.lock().await;
        guard.watch(dir.path()).unwrap();
        assert!(guard.watchers.contains_key(dir.path()));
        guard.unwatch(dir.path());
        assert!(!guard.watchers.contains_key(dir.path()));
    }

    #[tokio::test]
    async fn test_is_ignored_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();

        // Write a .gitignore that ignores *.log
        let mut f = std::fs::File::create(dir.path().join(".gitignore")).unwrap();
        writeln!(f, "*.log").unwrap();
        drop(f);

        // Create the files so is_dir() works correctly.
        std::fs::write(dir.path().join("debug.log"), "log").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main(){}").unwrap();

        let index = FileIndex::new();
        let (fw, _rx) = FileWatcher::new(index);
        let mut guard = fw.lock().await;
        guard.watch(dir.path()).unwrap();

        assert!(
            guard.is_ignored(&dir.path().join("debug.log")),
            "debug.log should be ignored"
        );
        assert!(
            !guard.is_ignored(&dir.path().join("main.rs")),
            "main.rs should not be ignored"
        );
    }

    #[tokio::test]
    async fn test_process_events_updates_index() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("hello.rs");
        std::fs::write(&file_path, "fn main() {}").unwrap();

        let index = FileIndex::new();
        let (fw, rx) = FileWatcher::new(index.clone());

        // Manually inject a Created event via the channel (bypassing notify).
        {
            let guard = fw.lock().await;
            guard
                .tx
                .send(WatchEvent::Created(file_path.clone()))
                .await
                .unwrap();
        }

        let fw_clone = fw.clone();
        let index_clone = index.clone();
        let symbol_index = crate::symbol_index::SymbolIndex::new();
        let handle = tokio::spawn(process_events(
            index_clone,
            symbol_index,
            fw_clone,
            empty_lsp_manager(),
            rx,
        ));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        handle.abort();

        assert_eq!(index.len().await, 1);
        assert!(index.get(&file_path).await.is_some());
    }

    #[tokio::test]
    async fn test_process_events_removes_from_index() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("hello.rs");
        std::fs::write(&file_path, "fn main() {}").unwrap();

        let index = FileIndex::new();
        index.update_file(&file_path).await.unwrap();
        assert_eq!(index.len().await, 1);

        let (fw, rx) = FileWatcher::new(index.clone());

        {
            let guard = fw.lock().await;
            guard
                .tx
                .send(WatchEvent::Removed(file_path.clone()))
                .await
                .unwrap();
        }

        let fw_clone = fw.clone();
        let index_clone = index.clone();
        let symbol_index = crate::symbol_index::SymbolIndex::new();
        let handle = tokio::spawn(process_events(
            index_clone,
            symbol_index,
            fw_clone,
            empty_lsp_manager(),
            rx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        handle.abort();

        assert_eq!(index.len().await, 0);
    }

    #[tokio::test]
    async fn test_process_events_updates_symbol_index() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("hello.rs");
        std::fs::write(&file_path, "fn greeting() {}").unwrap();

        let index = FileIndex::new();
        let symbol_index = crate::symbol_index::SymbolIndex::new();
        let (fw, rx) = FileWatcher::new(index.clone());

        {
            let guard = fw.lock().await;
            guard
                .tx
                .send(WatchEvent::Created(file_path.clone()))
                .await
                .unwrap();
        }

        let handle = tokio::spawn(process_events(
            index.clone(),
            symbol_index.clone(),
            fw.clone(),
            empty_lsp_manager(),
            rx,
        ));

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        handle.abort();

        let symbols = symbol_index.list_file_symbols(&file_path).await;
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greeting");
    }

    #[tokio::test]
    async fn test_process_events_removes_symbols_on_delete() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("hello.rs");
        std::fs::write(&file_path, "fn greeting() {}").unwrap();

        let index = FileIndex::new();
        let symbol_index = crate::symbol_index::SymbolIndex::new();
        index.update_file(&file_path).await.unwrap();
        {
            let si = symbol_index.clone();
            let p = file_path.clone();
            tokio::task::spawn_blocking(move || si.reparse_file(&p).unwrap())
                .await
                .unwrap();
        }
        assert_eq!(symbol_index.list_file_symbols(&file_path).await.len(), 1);

        let (fw, rx) = FileWatcher::new(index.clone());
        {
            let guard = fw.lock().await;
            guard
                .tx
                .send(WatchEvent::Removed(file_path.clone()))
                .await
                .unwrap();
        }

        let handle = tokio::spawn(process_events(
            index.clone(),
            symbol_index.clone(),
            fw.clone(),
            empty_lsp_manager(),
            rx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        handle.abort();

        assert!(symbol_index.list_file_symbols(&file_path).await.is_empty());
    }
}
