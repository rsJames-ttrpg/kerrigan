use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct LspDiagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Info = 3,
    Hint = 4,
}

impl DiagnosticSeverity {
    pub fn from_lsp(value: u64) -> Self {
        match value {
            1 => Self::Error,
            2 => Self::Warning,
            3 => Self::Info,
            _ => Self::Hint,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Hint => "hint",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "error" => Self::Error,
            "warning" => Self::Warning,
            "info" => Self::Info,
            _ => Self::Hint,
        }
    }
}

#[derive(Clone)]
pub struct DiagnosticsCache {
    entries: Arc<RwLock<BTreeMap<PathBuf, Vec<LspDiagnostic>>>>,
}

impl DiagnosticsCache {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub fn update(&self, file: PathBuf, diagnostics: Vec<LspDiagnostic>) {
        let mut entries = self.entries.write().unwrap();
        if diagnostics.is_empty() {
            entries.remove(&file);
        } else {
            entries.insert(file, diagnostics);
        }
    }

    pub fn get_all(&self, min_severity: DiagnosticSeverity) -> Vec<LspDiagnostic> {
        let entries = self.entries.read().unwrap();
        entries
            .values()
            .flat_map(|v| v.iter())
            .filter(|d| d.severity <= min_severity)
            .cloned()
            .collect()
    }

    pub fn get_file(&self, file: &PathBuf) -> Vec<LspDiagnostic> {
        let entries = self.entries.read().unwrap();
        entries.get(file).cloned().unwrap_or_default()
    }

    pub fn clear(&self) {
        let mut entries = self.entries.write().unwrap();
        entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_diagnostic(file: &str, line: u32, severity: DiagnosticSeverity, msg: &str) -> LspDiagnostic {
        LspDiagnostic {
            file: PathBuf::from(file),
            line,
            column: 0,
            severity,
            message: msg.to_string(),
            source: None,
        }
    }

    #[test]
    fn test_update_and_retrieve() {
        let cache = DiagnosticsCache::new();
        let file = PathBuf::from("/src/main.rs");
        let diags = vec![
            make_diagnostic("/src/main.rs", 10, DiagnosticSeverity::Error, "undefined variable"),
        ];
        cache.update(file.clone(), diags);

        let result = cache.get_file(&file);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].message, "undefined variable");
    }

    #[test]
    fn test_empty_diagnostics_clears_entry() {
        let cache = DiagnosticsCache::new();
        let file = PathBuf::from("/src/main.rs");
        cache.update(
            file.clone(),
            vec![make_diagnostic("/src/main.rs", 1, DiagnosticSeverity::Error, "err")],
        );
        assert_eq!(cache.get_file(&file).len(), 1);

        cache.update(file.clone(), vec![]);
        assert!(cache.get_file(&file).is_empty());
    }

    #[test]
    fn test_severity_filtering() {
        let cache = DiagnosticsCache::new();
        let file = PathBuf::from("/src/main.rs");
        cache.update(
            file.clone(),
            vec![
                make_diagnostic("/src/main.rs", 1, DiagnosticSeverity::Error, "error"),
                make_diagnostic("/src/main.rs", 2, DiagnosticSeverity::Warning, "warning"),
                make_diagnostic("/src/main.rs", 3, DiagnosticSeverity::Info, "info"),
                make_diagnostic("/src/main.rs", 4, DiagnosticSeverity::Hint, "hint"),
            ],
        );

        // Only errors
        let errors = cache.get_all(DiagnosticSeverity::Error);
        assert_eq!(errors.len(), 1);

        // Errors + warnings
        let warnings = cache.get_all(DiagnosticSeverity::Warning);
        assert_eq!(warnings.len(), 2);

        // All
        let all = cache.get_all(DiagnosticSeverity::Hint);
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn test_get_all_across_files() {
        let cache = DiagnosticsCache::new();
        cache.update(
            PathBuf::from("/a.rs"),
            vec![make_diagnostic("/a.rs", 1, DiagnosticSeverity::Error, "err1")],
        );
        cache.update(
            PathBuf::from("/b.rs"),
            vec![make_diagnostic("/b.rs", 2, DiagnosticSeverity::Error, "err2")],
        );

        let all = cache.get_all(DiagnosticSeverity::Hint);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(DiagnosticSeverity::Error < DiagnosticSeverity::Warning);
        assert!(DiagnosticSeverity::Warning < DiagnosticSeverity::Info);
        assert!(DiagnosticSeverity::Info < DiagnosticSeverity::Hint);
    }

    #[test]
    fn test_severity_from_lsp() {
        assert_eq!(DiagnosticSeverity::from_lsp(1), DiagnosticSeverity::Error);
        assert_eq!(DiagnosticSeverity::from_lsp(2), DiagnosticSeverity::Warning);
        assert_eq!(DiagnosticSeverity::from_lsp(3), DiagnosticSeverity::Info);
        assert_eq!(DiagnosticSeverity::from_lsp(4), DiagnosticSeverity::Hint);
        assert_eq!(DiagnosticSeverity::from_lsp(99), DiagnosticSeverity::Hint);
    }

    #[test]
    fn test_severity_as_str() {
        assert_eq!(DiagnosticSeverity::Error.as_str(), "error");
        assert_eq!(DiagnosticSeverity::Warning.as_str(), "warning");
        assert_eq!(DiagnosticSeverity::Info.as_str(), "info");
        assert_eq!(DiagnosticSeverity::Hint.as_str(), "hint");
    }

    #[test]
    fn test_clear() {
        let cache = DiagnosticsCache::new();
        cache.update(
            PathBuf::from("/a.rs"),
            vec![make_diagnostic("/a.rs", 1, DiagnosticSeverity::Error, "err")],
        );
        assert_eq!(cache.get_all(DiagnosticSeverity::Hint).len(), 1);
        cache.clear();
        assert!(cache.get_all(DiagnosticSeverity::Hint).is_empty());
    }
}
