use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

use crate::lsp::diagnostics::DiagnosticSeverity;
use crate::lsp::manager::LspManager;
use crate::proto::lsp_service_server::LspService as LspServiceTrait;
use crate::proto::{
    Diagnostic as ProtoDiagnostic, FindReferencesRequest, FindReferencesResponse,
    GetDiagnosticsRequest, GetDiagnosticsResponse, GetFileDiagnosticsRequest,
    GetFileDiagnosticsResponse, GotoDefinitionRequest, GotoDefinitionResponse,
    SymbolLocation as ProtoSymbolLocation,
};

/// gRPC service implementation for the `creep.v1.LspService` service.
#[derive(Clone)]
pub struct LspServiceImpl {
    pub lsp_manager: Arc<Mutex<LspManager>>,
}

impl LspServiceImpl {
    pub fn new(lsp_manager: Arc<Mutex<LspManager>>) -> Self {
        Self { lsp_manager }
    }
}

#[tonic::async_trait]
impl LspServiceTrait for LspServiceImpl {
    async fn get_diagnostics(
        &self,
        request: Request<GetDiagnosticsRequest>,
    ) -> Result<Response<GetDiagnosticsResponse>, Status> {
        let req = request.into_inner();
        let workspace = PathBuf::from(&req.workspace_path);
        let min_severity = match req.min_severity {
            1 => DiagnosticSeverity::Error,
            2 => DiagnosticSeverity::Warning,
            3 => DiagnosticSeverity::Info,
            _ => DiagnosticSeverity::Hint,
        };

        let mgr = self.lsp_manager.lock().await;
        let all_diags = mgr.diagnostics(&workspace, min_severity);
        let total_count = all_diags.len() as u32;

        let max = if req.max_results > 0 {
            req.max_results as usize
        } else {
            all_diags.len()
        };

        let diagnostics = all_diags
            .into_iter()
            .take(max)
            .map(|d| ProtoDiagnostic {
                file_path: d.file.to_string_lossy().into_owned(),
                line: d.line,
                column: d.column,
                severity: d.severity.as_str().to_string(),
                message: d.message,
                source: d.source.unwrap_or_default(),
            })
            .collect();

        Ok(Response::new(GetDiagnosticsResponse {
            diagnostics,
            total_count,
        }))
    }

    async fn get_file_diagnostics(
        &self,
        request: Request<GetFileDiagnosticsRequest>,
    ) -> Result<Response<GetFileDiagnosticsResponse>, Status> {
        let req = request.into_inner();
        let file = PathBuf::from(&req.file_path);

        let mgr = self.lsp_manager.lock().await;
        let diags = mgr.file_diagnostics(&file);

        let diagnostics = diags
            .into_iter()
            .map(|d| ProtoDiagnostic {
                file_path: d.file.to_string_lossy().into_owned(),
                line: d.line,
                column: d.column,
                severity: d.severity.as_str().to_string(),
                message: d.message,
                source: d.source.unwrap_or_default(),
            })
            .collect();

        Ok(Response::new(GetFileDiagnosticsResponse { diagnostics }))
    }

    async fn goto_definition(
        &self,
        request: Request<GotoDefinitionRequest>,
    ) -> Result<Response<GotoDefinitionResponse>, Status> {
        let req = request.into_inner();
        let file_path = PathBuf::from(&req.file_path);
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();

        let mut mgr = self.lsp_manager.lock().await;
        let workspace = mgr
            .find_workspace_for_file(&file_path)
            .ok_or_else(|| Status::not_found("no workspace registered for this file"))?;

        let client = mgr
            .ensure_server(&workspace, &ext)
            .await
            .map_err(|e| Status::internal(format!("failed to start LSP server: {e}")))?
            .ok_or_else(|| {
                Status::unimplemented(format!("no LSP server configured for extension '{ext}'"))
            })?;

        let locations = client
            .goto_definition(&req.file_path, req.line, req.column)
            .await
            .map_err(|e| Status::internal(format!("goto_definition failed: {e}")))?;

        let proto_locations = locations
            .into_iter()
            .map(|loc| ProtoSymbolLocation {
                file_path: loc.file.to_string_lossy().into_owned(),
                start_line: loc.start_line,
                start_column: loc.start_column,
                end_line: loc.end_line,
                end_column: loc.end_column,
            })
            .collect();

        Ok(Response::new(GotoDefinitionResponse {
            locations: proto_locations,
        }))
    }

    async fn find_references(
        &self,
        request: Request<FindReferencesRequest>,
    ) -> Result<Response<FindReferencesResponse>, Status> {
        let req = request.into_inner();
        let file_path = PathBuf::from(&req.file_path);
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();

        let mut mgr = self.lsp_manager.lock().await;
        let workspace = mgr
            .find_workspace_for_file(&file_path)
            .ok_or_else(|| Status::not_found("no workspace registered for this file"))?;

        let client = mgr
            .ensure_server(&workspace, &ext)
            .await
            .map_err(|e| Status::internal(format!("failed to start LSP server: {e}")))?
            .ok_or_else(|| {
                Status::unimplemented(format!("no LSP server configured for extension '{ext}'"))
            })?;

        let locations = client
            .find_references(
                &req.file_path,
                req.line,
                req.column,
                req.include_declaration,
            )
            .await
            .map_err(|e| Status::internal(format!("find_references failed: {e}")))?;

        let proto_locations = locations
            .into_iter()
            .map(|loc| ProtoSymbolLocation {
                file_path: loc.file.to_string_lossy().into_owned(),
                start_line: loc.start_line,
                start_column: loc.start_column,
                end_line: loc.end_line,
                end_column: loc.end_column,
            })
            .collect();

        Ok(Response::new(FindReferencesResponse {
            locations: proto_locations,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_service() -> LspServiceImpl {
        let mgr = LspManager::new(vec![]);
        LspServiceImpl::new(Arc::new(Mutex::new(mgr)))
    }

    #[tokio::test]
    async fn test_get_diagnostics_empty() {
        let svc = make_service();
        let resp = svc
            .get_diagnostics(Request::new(GetDiagnosticsRequest {
                workspace_path: "/some/workspace".to_string(),
                min_severity: 4,
                max_results: 0,
            }))
            .await
            .unwrap();
        let inner = resp.into_inner();
        assert!(inner.diagnostics.is_empty());
        assert_eq!(inner.total_count, 0);
    }

    #[tokio::test]
    async fn test_get_file_diagnostics_empty() {
        let svc = make_service();
        let resp = svc
            .get_file_diagnostics(Request::new(GetFileDiagnosticsRequest {
                workspace_path: "/some/workspace".to_string(),
                file_path: "/some/file.rs".to_string(),
            }))
            .await
            .unwrap();
        assert!(resp.into_inner().diagnostics.is_empty());
    }

    #[tokio::test]
    async fn test_goto_definition_no_workspace() {
        let svc = make_service();
        let result = svc
            .goto_definition(Request::new(GotoDefinitionRequest {
                file_path: "/unknown/file.rs".to_string(),
                line: 0,
                column: 0,
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_find_references_no_workspace() {
        let svc = make_service();
        let result = svc
            .find_references(Request::new(FindReferencesRequest {
                file_path: "/unknown/file.rs".to_string(),
                line: 0,
                column: 0,
                include_declaration: false,
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }
}
