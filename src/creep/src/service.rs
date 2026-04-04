use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

use crate::index::FileIndex;
use crate::symbol_index::SymbolIndex;
use crate::watcher::FileWatcher;

// Import the generated proto types.  `proto` is declared in main.rs.
use crate::proto::file_index_server::FileIndex as FileIndexTrait;
use crate::proto::{
    FileMetadata as ProtoFileMetadata, GetFileMetadataRequest, GetFileMetadataResponse,
    ListFileSymbolsRequest, ListFileSymbolsResponse, RegisterWorkspaceRequest,
    RegisterWorkspaceResponse, SearchFilesRequest, SearchFilesResponse, SearchSymbolsRequest,
    SearchSymbolsResponse, SymbolInfo, UnregisterWorkspaceRequest, UnregisterWorkspaceResponse,
};

/// gRPC service implementation for the `creep.v1.FileIndex` service.
#[derive(Clone)]
pub struct FileIndexServiceImpl {
    pub index: FileIndex,
    pub symbol_index: SymbolIndex,
    pub watcher: Arc<Mutex<FileWatcher>>,
}

impl FileIndexServiceImpl {
    pub fn new(
        index: FileIndex,
        symbol_index: SymbolIndex,
        watcher: Arc<Mutex<FileWatcher>>,
    ) -> Self {
        Self {
            index,
            symbol_index,
            watcher,
        }
    }
}

/// Convert our internal `FileMetadata` to the proto type.
fn to_proto_metadata(m: crate::index::FileMetadata) -> ProtoFileMetadata {
    ProtoFileMetadata {
        path: m.path.to_string_lossy().into_owned(),
        size: m.size,
        modified_at: m.modified_at,
        file_type: m.file_type,
        content_hash: m.content_hash,
    }
}

fn to_proto_symbol(sym: &crate::parser::Symbol, file: &std::path::Path) -> SymbolInfo {
    SymbolInfo {
        name: sym.name.clone(),
        kind: sym.kind.as_str().to_string(),
        file: file.to_string_lossy().into_owned(),
        line: sym.line,
        end_line: sym.end_line,
        parent: sym.parent.clone(),
        signature: sym.signature.clone(),
    }
}

#[tonic::async_trait]
impl FileIndexTrait for FileIndexServiceImpl {
    /// Search files by glob pattern, optionally filtered by workspace and file type.
    async fn search_files(
        &self,
        request: Request<SearchFilesRequest>,
    ) -> Result<Response<SearchFilesResponse>, Status> {
        let req = request.into_inner();
        let workspace = req.workspace.as_deref().map(PathBuf::from);
        let file_type = req.file_type.as_deref().map(str::to_owned);

        let results = self
            .index
            .search(&req.pattern, workspace.as_deref(), file_type.as_deref())
            .await;

        let files = results.into_iter().map(to_proto_metadata).collect();
        Ok(Response::new(SearchFilesResponse { files }))
    }

    /// Return metadata for a single file path.
    async fn get_file_metadata(
        &self,
        request: Request<GetFileMetadataRequest>,
    ) -> Result<Response<GetFileMetadataResponse>, Status> {
        let req = request.into_inner();
        let path = PathBuf::from(&req.path);
        let file = self.index.get(&path).await.map(to_proto_metadata);
        Ok(Response::new(GetFileMetadataResponse { file }))
    }

    /// Register a workspace: start watching it and scan all files.
    async fn register_workspace(
        &self,
        request: Request<RegisterWorkspaceRequest>,
    ) -> Result<Response<RegisterWorkspaceResponse>, Status> {
        let req = request.into_inner();

        if req.path.is_empty() {
            return Err(Status::invalid_argument("workspace path must not be empty"));
        }
        let path = PathBuf::from(&req.path);
        if !path.is_absolute() {
            return Err(Status::invalid_argument("workspace path must be absolute"));
        }
        if !path.is_dir() {
            return Err(Status::not_found(format!(
                "{} is not a directory",
                path.display()
            )));
        }

        // Start watching before scanning so no events are lost.
        {
            let mut guard = self.watcher.lock().await;
            guard.watch(&path).map_err(|e| {
                Status::internal(format!("failed to watch {}: {e}", path.display()))
            })?;
        }

        let files_indexed = match self.index.scan_workspace(&path).await {
            Ok(n) => n,
            Err(e) => {
                let mut guard = self.watcher.lock().await;
                guard.unwatch(&path);
                return Err(Status::internal(format!(
                    "failed to scan workspace {}: {e}",
                    path.display()
                )));
            }
        };

        // Scan symbols in the new workspace (blocking — tree-sitter is sync).
        let si = self.symbol_index.clone();
        let ws = path.clone();
        match tokio::task::spawn_blocking(move || si.scan_workspace(&ws)).await {
            Ok(Ok(n)) => tracing::info!("parsed {n} symbols in {}", path.display()),
            Ok(Err(e)) => tracing::warn!("symbol scan failed for {}: {e}", path.display()),
            Err(e) => tracing::warn!("symbol scan task panicked for {}: {e}", path.display()),
        }

        tracing::info!(
            "registered workspace {} ({files_indexed} files)",
            path.display()
        );

        Ok(Response::new(RegisterWorkspaceResponse { files_indexed }))
    }

    /// Unregister a workspace: stop watching it (indexed files remain until evicted).
    async fn unregister_workspace(
        &self,
        request: Request<UnregisterWorkspaceRequest>,
    ) -> Result<Response<UnregisterWorkspaceResponse>, Status> {
        let req = request.into_inner();
        let path = PathBuf::from(&req.path);

        {
            let mut guard = self.watcher.lock().await;
            guard.unwatch(&path);
        }

        tracing::info!("unregistered workspace {}", path.display());
        Ok(Response::new(UnregisterWorkspaceResponse {}))
    }

    async fn search_symbols(
        &self,
        request: Request<SearchSymbolsRequest>,
    ) -> Result<Response<SearchSymbolsResponse>, Status> {
        let req = request.into_inner();
        let kind = req
            .kind
            .as_deref()
            .and_then(|s| s.parse::<crate::parser::SymbolKind>().ok());
        let workspace = req.workspace.as_deref().map(std::path::PathBuf::from);

        let results = self
            .symbol_index
            .search(&req.query, kind.as_ref(), workspace.as_deref())
            .await;

        let symbols = results
            .iter()
            .map(|(sym, path)| to_proto_symbol(sym, path))
            .collect();

        Ok(Response::new(SearchSymbolsResponse { symbols }))
    }

    async fn list_file_symbols(
        &self,
        request: Request<ListFileSymbolsRequest>,
    ) -> Result<Response<ListFileSymbolsResponse>, Status> {
        let req = request.into_inner();
        let path = std::path::PathBuf::from(&req.path);

        let symbols = self
            .symbol_index
            .list_file_symbols(&path)
            .await
            .iter()
            .map(|sym| to_proto_symbol(sym, &path))
            .collect();

        Ok(Response::new(ListFileSymbolsResponse { symbols }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{
        GetFileMetadataRequest, RegisterWorkspaceRequest, SearchFilesRequest,
        UnregisterWorkspaceRequest,
    };

    fn make_service() -> FileIndexServiceImpl {
        let index = FileIndex::new();
        let symbol_index = crate::symbol_index::SymbolIndex::new();
        let (watcher, _rx) = FileWatcher::new(index.clone());
        FileIndexServiceImpl::new(index, symbol_index, watcher)
    }

    #[tokio::test]
    async fn test_register_and_search() {
        // tempfile creates dirs under /tmp/.tmpXXX (hidden dir) which is skipped
        // by ignore::WalkBuilder.  Use a non-hidden path under /tmp directly.
        let base = std::env::temp_dir().join("creep_svc_test_reg_search");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(base.join("lib.rs"), "pub fn foo() {}").unwrap();

        let svc = make_service();

        // Register workspace.
        let resp = svc
            .register_workspace(Request::new(RegisterWorkspaceRequest {
                path: base.to_string_lossy().into_owned(),
            }))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().files_indexed, 2);

        // Search for .rs files.
        let resp = svc
            .search_files(Request::new(SearchFilesRequest {
                pattern: "**/*.rs".to_string(),
                workspace: None,
                file_type: None,
            }))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().files.len(), 2);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn test_get_file_metadata_found() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("foo.rs");
        std::fs::write(&file, "fn foo() {}").unwrap();

        let svc = make_service();
        svc.index.update_file(&file).await.unwrap();

        let resp = svc
            .get_file_metadata(Request::new(GetFileMetadataRequest {
                path: file.to_string_lossy().into_owned(),
            }))
            .await
            .unwrap();

        let inner = resp.into_inner();
        assert!(inner.file.is_some());
        let meta = inner.file.unwrap();
        assert_eq!(meta.file_type, "rust");
        assert_eq!(meta.path, file.to_string_lossy().as_ref());
    }

    #[tokio::test]
    async fn test_get_file_metadata_not_found() {
        let svc = make_service();
        let resp = svc
            .get_file_metadata(Request::new(GetFileMetadataRequest {
                path: "/nonexistent/file.rs".to_string(),
            }))
            .await
            .unwrap();
        assert!(resp.into_inner().file.is_none());
    }

    #[tokio::test]
    async fn test_unregister_workspace() {
        let base = std::env::temp_dir().join("creep_svc_test_unreg");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let svc = make_service();

        // Register first so watcher state exists.
        svc.register_workspace(Request::new(RegisterWorkspaceRequest {
            path: base.to_string_lossy().into_owned(),
        }))
        .await
        .unwrap();

        // Unregister should succeed without error.
        let resp = svc
            .unregister_workspace(Request::new(UnregisterWorkspaceRequest {
                path: base.to_string_lossy().into_owned(),
            }))
            .await
            .unwrap();
        let _ = resp.into_inner();

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn test_search_by_file_type() {
        // Use a non-hidden path to avoid ignore::WalkBuilder skipping hidden dirs.
        let base = std::env::temp_dir().join("creep_svc_test_file_type");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("app.py"), "print('hi')").unwrap();
        std::fs::write(base.join("main.rs"), "fn main() {}").unwrap();

        let svc = make_service();
        svc.register_workspace(Request::new(RegisterWorkspaceRequest {
            path: base.to_string_lossy().into_owned(),
        }))
        .await
        .unwrap();

        let resp = svc
            .search_files(Request::new(SearchFilesRequest {
                pattern: "**/*".to_string(),
                workspace: None,
                file_type: Some("python".to_string()),
            }))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().files.len(), 1);

        let _ = std::fs::remove_dir_all(&base);
    }
}
