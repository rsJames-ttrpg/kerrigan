/// Parses structured markdown plans into task dependency graphs.
///
/// Expected format:
/// ```markdown
/// - [ ] **task-id**: Description of the task
///   - Files: src/foo.rs, src/bar.rs
///   - Depends: task-other, task-another
/// ```

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub dependencies: Vec<String>,
    pub files: Vec<String>,
}

/// Parse a structured markdown plan into a list of tasks.
pub fn parse_plan(markdown: &str) -> Vec<Task> {
    let mut tasks = Vec::new();
    let mut current_task: Option<Task> = None;

    for line in markdown.lines() {
        let trimmed = line.trim();

        if let Some(task_match) = parse_task_line(trimmed) {
            if let Some(task) = current_task.take() {
                tasks.push(task);
            }
            current_task = Some(task_match);
            continue;
        }

        if let Some(task) = current_task.as_mut() {
            if let Some(files) = trimmed
                .strip_prefix("- Files:")
                .or_else(|| trimmed.strip_prefix("- files:"))
            {
                task.files = files
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            if let Some(deps) = trimmed
                .strip_prefix("- Depends:")
                .or_else(|| trimmed.strip_prefix("- depends:"))
            {
                let deps_str = deps.trim();
                if deps_str != "none" && !deps_str.is_empty() {
                    task.dependencies = deps_str
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }
    }

    if let Some(task) = current_task {
        tasks.push(task);
    }

    tasks
}

fn parse_task_line(line: &str) -> Option<Task> {
    // Match: - [ ] **task-id**: description
    let line = line.strip_prefix("- [ ] ")?;
    let line = line.strip_prefix("**")?;
    let (id, rest) = line.split_once("**")?;
    let description = rest.strip_prefix(": ")?.trim().to_string();
    Some(Task {
        id: id.to_string(),
        description,
        dependencies: Vec::new(),
        files: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_task() {
        let md = "- [ ] **task-1**: Do something\n  - Files: src/main.rs\n  - Depends: none\n";
        let tasks = parse_plan(md);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "task-1");
        assert_eq!(tasks[0].description, "Do something");
        assert_eq!(tasks[0].files, vec!["src/main.rs"]);
        assert!(tasks[0].dependencies.is_empty());
    }

    #[test]
    fn test_parse_with_dependencies() {
        let md = "- [ ] **task-1**: First\n  - Depends: none\n\n- [ ] **task-2**: Second\n  - Depends: task-1\n";
        let tasks = parse_plan(md);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[1].dependencies, vec!["task-1"]);
    }

    #[test]
    fn test_parse_multiple_files() {
        let md = "- [ ] **task-1**: Thing\n  - Files: src/a.rs, src/b.rs, tests/c.rs\n";
        let tasks = parse_plan(md);
        assert_eq!(tasks[0].files.len(), 3);
        assert_eq!(tasks[0].files[0], "src/a.rs");
        assert_eq!(tasks[0].files[1], "src/b.rs");
        assert_eq!(tasks[0].files[2], "tests/c.rs");
    }

    #[test]
    fn test_parse_multiple_dependencies() {
        let md = "- [ ] **task-3**: Third\n  - Depends: task-1, task-2\n";
        let tasks = parse_plan(md);
        assert_eq!(tasks[0].dependencies, vec!["task-1", "task-2"]);
    }

    #[test]
    fn test_parse_empty_input() {
        let tasks = parse_plan("");
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_no_tasks_in_markdown() {
        let md = "# Plan\n\nSome description text.\n\n## Notes\n\nNothing here.\n";
        let tasks = parse_plan(md);
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_task_without_files_or_deps() {
        let md = "- [ ] **task-1**: Standalone task\n";
        let tasks = parse_plan(md);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "task-1");
        assert!(tasks[0].files.is_empty());
        assert!(tasks[0].dependencies.is_empty());
    }

    #[test]
    fn test_parse_ignores_surrounding_markdown() {
        let md = r#"# Implementation Plan

## Overview
This plan implements feature X.

## Tasks

- [ ] **task-1**: Add auth middleware to axum router
  - Files: src/api/mod.rs, src/api/auth.rs
  - Depends: none

- [ ] **task-2**: Write auth middleware tests
  - Files: src/api/auth.rs, tests/api_auth.rs
  - Depends: task-1

## Notes
Some trailing notes.
"#;
        let tasks = parse_plan(md);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "task-1");
        assert_eq!(tasks[0].description, "Add auth middleware to axum router");
        assert_eq!(tasks[1].id, "task-2");
        assert_eq!(tasks[1].dependencies, vec!["task-1"]);
    }

    #[test]
    fn test_parse_case_insensitive_prefixes() {
        let md = "- [ ] **task-1**: Thing\n  - files: src/a.rs\n  - depends: task-0\n";
        let tasks = parse_plan(md);
        assert_eq!(tasks[0].files, vec!["src/a.rs"]);
        assert_eq!(tasks[0].dependencies, vec!["task-0"]);
    }
}
