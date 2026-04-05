use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use async_trait::async_trait;
use globset::GlobBuilder;
use regex::Regex;

use super::registry::Tool;
use super::types::*;

/// Validate that a path is within the workspace. Returns canonical path on success.
fn validate_path(workspace: &Path, file_path: &str) -> Result<PathBuf, String> {
    let path = if Path::new(file_path).is_absolute() {
        PathBuf::from(file_path)
    } else {
        workspace.join(file_path)
    };

    // Canonicalize workspace for comparison (it should exist)
    let canonical_workspace = workspace
        .canonicalize()
        .map_err(|e| format!("cannot resolve workspace: {e}"))?;

    // For existing files, canonicalize. For new files, canonicalize parent.
    let canonical = if path.exists() {
        path.canonicalize()
            .map_err(|e| format!("cannot resolve path: {e}"))?
    } else {
        let parent = path.parent().ok_or("invalid path: no parent")?;
        if parent.exists() {
            let canonical_parent = parent
                .canonicalize()
                .map_err(|e| format!("cannot resolve parent: {e}"))?;
            canonical_parent.join(path.file_name().ok_or("invalid path: no filename")?)
        } else {
            // Parent doesn't exist yet — we'll create it. Check the closest existing ancestor.
            let mut ancestor = parent.to_path_buf();
            while !ancestor.exists() {
                ancestor = ancestor
                    .parent()
                    .ok_or("invalid path: no existing ancestor")?
                    .to_path_buf();
            }
            let canonical_ancestor = ancestor
                .canonicalize()
                .map_err(|e| format!("cannot resolve ancestor: {e}"))?;
            if !canonical_ancestor.starts_with(&canonical_workspace) {
                return Err(format!("path escapes workspace: {}", path.display()));
            }
            // Return the original path (not fully canonical since parent doesn't exist yet)
            return Ok(path);
        }
    };

    if !canonical.starts_with(&canonical_workspace) {
        return Err(format!("path escapes workspace: {}", path.display()));
    }

    Ok(canonical)
}

// --- ReadFileTool ---

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file from the workspace with optional line offset and limit"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["file_path"],
            "properties": {
                "file_path": { "type": "string", "description": "Path to the file to read" },
                "offset": { "type": "integer", "description": "Line number to start from (0-based)" },
                "limit": { "type": "integer", "description": "Maximum number of lines to read" }
            }
        })
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let file_path = match input["file_path"].as_str() {
            Some(p) => p,
            None => return ToolResult::error("missing required field: file_path".into()),
        };

        let path = match validate_path(&ctx.workspace, file_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(e),
        };

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("cannot read file: {e}")),
        };

        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().map(|l| l as usize);

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        let end = match limit {
            Some(l) => (offset + l).min(total),
            None => total,
        };

        let start = offset.min(total);

        let mut output = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;
            output.push_str(&format!("{line_num}\t{line}\n"));
        }

        ToolResult::success(output)
    }
}

// --- WriteFileTool ---

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file in the workspace, creating parent directories as needed"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["file_path", "content"],
            "properties": {
                "file_path": { "type": "string", "description": "Path to the file to write" },
                "content": { "type": "string", "description": "Content to write to the file" }
            }
        })
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::WorkspaceWrite
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let file_path = match input["file_path"].as_str() {
            Some(p) => p,
            None => return ToolResult::error("missing required field: file_path".into()),
        };
        let content = match input["content"].as_str() {
            Some(c) => c,
            None => return ToolResult::error("missing required field: content".into()),
        };

        let path = match validate_path(&ctx.workspace, file_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(e),
        };

        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                return ToolResult::error(format!("cannot create directories: {e}"));
            }
        }

        match fs::write(&path, content) {
            Ok(()) => {
                ToolResult::success(format!("wrote {} bytes to {}", content.len(), file_path))
            }
            Err(e) => ToolResult::error(format!("cannot write file: {e}")),
        }
    }
}

// --- EditFileTool ---

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact string match. Fails if old_string is not found or is ambiguous (unless replace_all is set)."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["file_path", "old_string", "new_string"],
            "properties": {
                "file_path": { "type": "string", "description": "Path to the file to edit" },
                "old_string": { "type": "string", "description": "Exact string to find and replace" },
                "new_string": { "type": "string", "description": "Replacement string" },
                "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)" }
            }
        })
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::WorkspaceWrite
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let file_path = match input["file_path"].as_str() {
            Some(p) => p,
            None => return ToolResult::error("missing required field: file_path".into()),
        };
        let old_string = match input["old_string"].as_str() {
            Some(s) => s,
            None => return ToolResult::error("missing required field: old_string".into()),
        };
        let new_string = match input["new_string"].as_str() {
            Some(s) => s,
            None => return ToolResult::error("missing required field: new_string".into()),
        };
        let replace_all = input["replace_all"].as_bool().unwrap_or(false);

        let path = match validate_path(&ctx.workspace, file_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(e),
        };

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("cannot read file: {e}")),
        };

        let count = content.matches(old_string).count();

        if count == 0 {
            return ToolResult::error(format!("old_string not found in {file_path}"));
        }

        if count > 1 && !replace_all {
            return ToolResult::error(format!(
                "old_string found {count} times in {file_path} — use replace_all to replace all occurrences"
            ));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        match fs::write(&path, &new_content) {
            Ok(()) => ToolResult::success(format!("replaced {count} occurrence(s) in {file_path}")),
            Err(e) => ToolResult::error(format!("cannot write file: {e}")),
        }
    }
}

// --- GlobSearchTool ---

pub struct GlobSearchTool;

#[async_trait]
impl Tool for GlobSearchTool {
    fn name(&self) -> &str {
        "glob_search"
    }

    fn description(&self) -> &str {
        "Search for files matching a glob pattern in the workspace"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern to match files" },
                "path": { "type": "string", "description": "Subdirectory to search in (default: workspace root)" }
            }
        })
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let pattern = match input["pattern"].as_str() {
            Some(p) => p,
            None => return ToolResult::error("missing required field: pattern".into()),
        };

        let search_root = match input["path"].as_str() {
            Some(p) => match validate_path(&ctx.workspace, p) {
                Ok(path) => path,
                Err(e) => return ToolResult::error(e),
            },
            None => ctx.workspace.clone(),
        };

        let glob = match GlobBuilder::new(pattern).literal_separator(false).build() {
            Ok(g) => g.compile_matcher(),
            Err(e) => return ToolResult::error(format!("invalid glob pattern: {e}")),
        };

        let mut matches: Vec<(PathBuf, SystemTime)> = Vec::new();
        collect_glob_matches(&search_root, &glob, &mut matches);

        // Sort by modification time (most recent first)
        matches.sort_by(|a, b| b.1.cmp(&a.1));

        let mut output = String::new();
        for (path, _) in &matches {
            let display = path.strip_prefix(&ctx.workspace).unwrap_or(path).display();
            output.push_str(&format!("{display}\n"));
        }

        if matches.is_empty() {
            ToolResult::success("no matches found".into())
        } else {
            ToolResult::success(output)
        }
    }
}

fn collect_glob_matches(
    dir: &Path,
    glob: &globset::GlobMatcher,
    results: &mut Vec<(PathBuf, SystemTime)>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Skip hidden directories and common ignore patterns
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
        }

        if path.is_dir() {
            collect_glob_matches(&path, glob, results);
        } else {
            // Match against the relative path from the search root
            let rel = path
                .strip_prefix(dir.ancestors().last().unwrap_or(dir))
                .unwrap_or(&path);
            if glob.is_match(rel) || glob.is_match(path.file_name().unwrap_or_default()) {
                let mtime = path
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                results.push((path.clone(), mtime));
            }
        }
    }
}

// --- GrepSearchTool ---

pub struct GrepSearchTool;

#[async_trait]
impl Tool for GrepSearchTool {
    fn name(&self) -> &str {
        "grep_search"
    }

    fn description(&self) -> &str {
        "Search file contents using a regex pattern"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search for" },
                "path": { "type": "string", "description": "File or directory to search in" },
                "glob": { "type": "string", "description": "Glob pattern to filter files" },
                "context": { "type": "integer", "description": "Number of context lines before and after each match" }
            }
        })
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let pattern = match input["pattern"].as_str() {
            Some(p) => p,
            None => return ToolResult::error("missing required field: pattern".into()),
        };

        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("invalid regex: {e}")),
        };

        let search_root = match input["path"].as_str() {
            Some(p) => match validate_path(&ctx.workspace, p) {
                Ok(path) => path,
                Err(e) => return ToolResult::error(e),
            },
            None => ctx.workspace.clone(),
        };

        let file_glob = input["glob"].as_str().and_then(|g| {
            GlobBuilder::new(g)
                .literal_separator(false)
                .build()
                .ok()
                .map(|g| g.compile_matcher())
        });

        let context_lines = input["context"].as_u64().unwrap_or(0) as usize;

        let mut output = String::new();
        let mut match_count = 0;

        if search_root.is_file() {
            grep_file(
                &search_root,
                &re,
                context_lines,
                &ctx.workspace,
                &mut output,
                &mut match_count,
            );
        } else {
            grep_dir(
                &search_root,
                &re,
                &file_glob,
                context_lines,
                &ctx.workspace,
                &mut output,
                &mut match_count,
            );
        }

        if match_count == 0 {
            ToolResult::success("no matches found".into())
        } else {
            ToolResult::success(format!("{output}\n{match_count} match(es) found"))
        }
    }
}

fn grep_file(
    path: &Path,
    re: &Regex,
    context: usize,
    workspace: &Path,
    output: &mut String,
    match_count: &mut usize,
) {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };

    // Skip binary files
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().take(10_000).filter_map(|l| l.ok()).collect();

    let mut matched_ranges: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            matched_ranges.push(i);
            *match_count += 1;
        }
    }

    if matched_ranges.is_empty() {
        return;
    }

    let display_path = path.strip_prefix(workspace).unwrap_or(path);
    output.push_str(&format!("## {}\n", display_path.display()));

    for &match_idx in &matched_ranges {
        let start = match_idx.saturating_sub(context);
        let end = (match_idx + context + 1).min(lines.len());
        for i in start..end {
            let marker = if i == match_idx { ">" } else { " " };
            output.push_str(&format!("{marker} {}:{}\n", i + 1, lines[i]));
        }
        output.push('\n');
    }
}

fn grep_dir(
    dir: &Path,
    re: &Regex,
    file_glob: &Option<globset::GlobMatcher>,
    context: usize,
    workspace: &Path,
    output: &mut String,
    match_count: &mut usize,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
        }

        if path.is_dir() {
            grep_dir(
                &path,
                re,
                file_glob,
                context,
                workspace,
                output,
                match_count,
            );
        } else {
            if let Some(glob) = &file_glob {
                let fname = path.file_name().unwrap_or_default();
                if !glob.is_match(fname) {
                    continue;
                }
            }
            grep_file(&path, re, context, workspace, output, match_count);
        }
    }
}

pub fn register_file_tools(registry: &mut super::ToolRegistry) {
    registry.register(Box::new(ReadFileTool));
    registry.register(Box::new(WriteFileTool));
    registry.register(Box::new(EditFileTool));
    registry.register(Box::new(GlobSearchTool));
    registry.register(Box::new(GrepSearchTool));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::NullEventSink;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn test_ctx(dir: &Path) -> ToolContext {
        ToolContext {
            workspace: dir.to_path_buf(),
            home: dir.to_path_buf(),
            event_sink: Arc::new(NullEventSink),
        }
    }

    // --- ReadFileTool tests ---

    #[tokio::test]
    async fn test_read_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\n").unwrap();
        let ctx = test_ctx(dir.path());

        let result = ReadFileTool
            .execute(
                serde_json::json!({"file_path": file.to_str().unwrap()}),
                &ctx,
            )
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("1\tline1"));
        assert!(result.output.contains("2\tline2"));
        assert!(result.output.contains("3\tline3"));
    }

    #[tokio::test]
    async fn test_read_file_with_offset_limit() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "a\nb\nc\nd\ne\n").unwrap();
        let ctx = test_ctx(dir.path());

        let result = ReadFileTool
            .execute(
                serde_json::json!({"file_path": file.to_str().unwrap(), "offset": 1, "limit": 2}),
                &ctx,
            )
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("2\tb"));
        assert!(result.output.contains("3\tc"));
        assert!(!result.output.contains("1\ta"));
        assert!(!result.output.contains("4\td"));
    }

    #[tokio::test]
    async fn test_read_file_outside_workspace() {
        let dir = TempDir::new().unwrap();
        let ctx = test_ctx(dir.path());

        let result = ReadFileTool
            .execute(serde_json::json!({"file_path": "/etc/passwd"}), &ctx)
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("escapes workspace"));
    }

    // --- WriteFileTool tests ---

    #[tokio::test]
    async fn test_write_file() {
        let dir = TempDir::new().unwrap();
        let ctx = test_ctx(dir.path());
        let file = dir.path().join("new.txt");

        let result = WriteFileTool
            .execute(
                serde_json::json!({"file_path": file.to_str().unwrap(), "content": "hello"}),
                &ctx,
            )
            .await;
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
    }

    #[tokio::test]
    async fn test_write_file_nested_dirs() {
        let dir = TempDir::new().unwrap();
        let ctx = test_ctx(dir.path());
        let file = dir.path().join("a/b/c.txt");

        let result = WriteFileTool
            .execute(
                serde_json::json!({"file_path": file.to_str().unwrap(), "content": "deep"}),
                &ctx,
            )
            .await;
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&file).unwrap(), "deep");
    }

    #[tokio::test]
    async fn test_write_file_outside_workspace() {
        let dir = TempDir::new().unwrap();
        let ctx = test_ctx(dir.path());

        let result = WriteFileTool
            .execute(
                serde_json::json!({"file_path": "/tmp/evil.txt", "content": "bad"}),
                &ctx,
            )
            .await;
        assert!(result.is_error);
    }

    // --- EditFileTool tests ---

    #[tokio::test]
    async fn test_edit_single_replacement() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("edit.txt");
        fs::write(&file, "hello world").unwrap();
        let ctx = test_ctx(dir.path());

        let result = EditFileTool
            .execute(
                serde_json::json!({
                    "file_path": file.to_str().unwrap(),
                    "old_string": "hello",
                    "new_string": "goodbye"
                }),
                &ctx,
            )
            .await;
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&file).unwrap(), "goodbye world");
    }

    #[tokio::test]
    async fn test_edit_replace_all() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("edit.txt");
        fs::write(&file, "aaa bbb aaa").unwrap();
        let ctx = test_ctx(dir.path());

        let result = EditFileTool
            .execute(
                serde_json::json!({
                    "file_path": file.to_str().unwrap(),
                    "old_string": "aaa",
                    "new_string": "ccc",
                    "replace_all": true
                }),
                &ctx,
            )
            .await;
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&file).unwrap(), "ccc bbb ccc");
    }

    #[tokio::test]
    async fn test_edit_not_found() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("edit.txt");
        fs::write(&file, "hello world").unwrap();
        let ctx = test_ctx(dir.path());

        let result = EditFileTool
            .execute(
                serde_json::json!({
                    "file_path": file.to_str().unwrap(),
                    "old_string": "missing",
                    "new_string": "x"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_ambiguous() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("edit.txt");
        fs::write(&file, "aaa bbb aaa").unwrap();
        let ctx = test_ctx(dir.path());

        let result = EditFileTool
            .execute(
                serde_json::json!({
                    "file_path": file.to_str().unwrap(),
                    "old_string": "aaa",
                    "new_string": "ccc"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("2 times"));
    }

    // --- GlobSearchTool tests ---

    #[tokio::test]
    async fn test_glob_match_rs_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("lib.rs"), "pub fn lib() {}").unwrap();
        fs::write(dir.path().join("readme.md"), "# readme").unwrap();
        let ctx = test_ctx(dir.path());

        let result = GlobSearchTool
            .execute(serde_json::json!({"pattern": "*.rs"}), &ctx)
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("main.rs"));
        assert!(result.output.contains("lib.rs"));
        assert!(!result.output.contains("readme.md"));
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.txt"), "").unwrap();
        let ctx = test_ctx(dir.path());

        let result = GlobSearchTool
            .execute(serde_json::json!({"pattern": "*.xyz"}), &ctx)
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("no matches"));
    }

    // --- GrepSearchTool tests ---

    #[tokio::test]
    async fn test_grep_simple_match() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("test.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        let ctx = test_ctx(dir.path());

        let result = GrepSearchTool
            .execute(serde_json::json!({"pattern": "println"}), &ctx)
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("println"));
        assert!(result.output.contains("1 match"));
    }

    #[tokio::test]
    async fn test_grep_regex_pattern() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("test.rs"),
            "fn foo() {}\nfn bar() {}\nfn baz() {}\n",
        )
        .unwrap();
        let ctx = test_ctx(dir.path());

        let result = GrepSearchTool
            .execute(serde_json::json!({"pattern": "fn ba[rz]"}), &ctx)
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("2 match"));
    }

    #[tokio::test]
    async fn test_grep_with_context() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.txt"), "aaa\nbbb\nccc\nddd\neee\n").unwrap();
        let ctx = test_ctx(dir.path());

        let result = GrepSearchTool
            .execute(serde_json::json!({"pattern": "ccc", "context": 1}), &ctx)
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("bbb"));
        assert!(result.output.contains("ccc"));
        assert!(result.output.contains("ddd"));
    }

    #[tokio::test]
    async fn test_grep_file_filter() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.rs"), "target_text\n").unwrap();
        fs::write(dir.path().join("test.txt"), "target_text\n").unwrap();
        let ctx = test_ctx(dir.path());

        let result = GrepSearchTool
            .execute(
                serde_json::json!({"pattern": "target_text", "glob": "*.rs"}),
                &ctx,
            )
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("test.rs"));
        assert!(!result.output.contains("test.txt"));
    }
}
