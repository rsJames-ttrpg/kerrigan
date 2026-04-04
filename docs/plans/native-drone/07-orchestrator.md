# Plan 07: Drone Orchestrator

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the task orchestrator that decomposes implementation plans into parallel sub-agent tasks, manages their execution, coordinates git commits, and handles failures.

**Architecture:** `Orchestrator` parses a structured markdown plan into a task dependency graph. It spawns sub-agents (via the runtime's agent spawning) for independent tasks, serializes git commits, and checkpoints after each task completion. Active during Implement stage; optional for Freeform.

**Tech Stack:** tokio (concurrent task spawning), runtime (ConversationLoop, sub-agents)

**Spec:** `docs/specs/native-drone/04-drone-pipeline.md` (Orchestrator section)

---

### Task 1: Plan parser

**Files:**
- Create: `src/drones/native/src/orchestrator/plan_parser.rs`
- Create: `src/drones/native/src/orchestrator/mod.rs`

- [ ] **Step 1: Define Task type**

```rust
#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub dependencies: Vec<String>,
    pub files: Vec<String>,
}
```

- [ ] **Step 2: Implement plan markdown parser with tests**

Parse structured markdown into `Vec<Task>`:

```markdown
## Tasks

- [ ] **task-1**: Add auth middleware to axum router
  - Files: src/api/mod.rs, src/api/auth.rs
  - Depends: none

- [ ] **task-2**: Write auth middleware tests
  - Files: src/api/auth.rs, tests/api_auth.rs
  - Depends: task-1
```

Parser:
```rust
pub fn parse_plan(markdown: &str) -> Vec<Task> {
    let mut tasks = Vec::new();
    let mut current_task: Option<Task> = None;

    for line in markdown.lines() {
        let trimmed = line.trim();

        // Match: - [ ] **task-id**: description
        if let Some(task_match) = parse_task_line(trimmed) {
            if let Some(task) = current_task.take() {
                tasks.push(task);
            }
            current_task = Some(task_match);
            continue;
        }

        if let Some(task) = current_task.as_mut() {
            // Match: - Files: path1, path2
            if let Some(files) = trimmed.strip_prefix("- Files:").or_else(|| trimmed.strip_prefix("- files:")) {
                task.files = files.split(',').map(|s| s.trim().to_string()).collect();
            }
            // Match: - Depends: task-1, task-2
            if let Some(deps) = trimmed.strip_prefix("- Depends:").or_else(|| trimmed.strip_prefix("- depends:")) {
                let deps_str = deps.trim();
                if deps_str != "none" && !deps_str.is_empty() {
                    task.dependencies = deps_str.split(',').map(|s| s.trim().to_string()).collect();
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
```

Tests:
```rust
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
}
```

- [ ] **Step 3: Run tests, commit**

```bash
git add src/drones/native/
git commit -m "add plan markdown parser for task decomposition"
```

---

### Task 2: Task dependency graph and scheduling

**Files:**
- Create: `src/drones/native/src/orchestrator/scheduler.rs`

- [ ] **Step 1: Implement topological sort and ready task detection**

```rust
use std::collections::{HashMap, HashSet, VecDeque};

pub struct TaskScheduler {
    tasks: Vec<Task>,
    completed: HashSet<String>,
    in_progress: HashSet<String>,
}

impl TaskScheduler {
    pub fn new(tasks: Vec<Task>) -> anyhow::Result<Self> {
        // Validate: no cycles, all dependencies exist
        validate_dag(&tasks)?;
        Ok(Self {
            tasks,
            completed: HashSet::new(),
            in_progress: HashSet::new(),
        })
    }

    /// Get tasks whose dependencies are all satisfied and not yet started
    pub fn ready_tasks(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| {
                !self.completed.contains(&t.id)
                    && !self.in_progress.contains(&t.id)
                    && t.dependencies.iter().all(|d| self.completed.contains(d))
            })
            .collect()
    }

    pub fn start(&mut self, task_id: &str) {
        self.in_progress.insert(task_id.to_string());
    }

    pub fn complete(&mut self, task_id: &str) {
        self.in_progress.remove(task_id);
        self.completed.insert(task_id.to_string());
    }

    pub fn is_done(&self) -> bool {
        self.completed.len() == self.tasks.len()
    }

    pub fn remaining(&self) -> usize {
        self.tasks.len() - self.completed.len()
    }
}

fn validate_dag(tasks: &[Task]) -> anyhow::Result<()> {
    let ids: HashSet<_> = tasks.iter().map(|t| t.id.as_str()).collect();
    for task in tasks {
        for dep in &task.dependencies {
            anyhow::ensure!(ids.contains(dep.as_str()), "unknown dependency: {dep} in task {}", task.id);
        }
    }
    // Cycle detection via topological sort
    // ... (Kahn's algorithm)
    Ok(())
}
```

Tests: ready tasks with no deps, ready tasks after completion, cycle detection, unknown dependency error.

- [ ] **Step 2: Run tests, commit**

```bash
git add src/drones/native/
git commit -m "add task dependency scheduler with topological ordering"
```

---

### Task 3: Orchestrator execution loop

**Files:**
- Create: `src/drones/native/src/orchestrator/executor.rs`
- Modify: `src/drones/native/src/orchestrator/mod.rs`

- [ ] **Step 1: Implement Orchestrator**

```rust
use std::sync::Arc;
use runtime::conversation::ConversationLoop;
use runtime::event::EventSink;
use crate::git_workflow::GitWorkflow;

pub struct Orchestrator {
    scheduler: TaskScheduler,
    max_parallel: usize,
    event_sink: Arc<dyn EventSink>,
    git_workflow: Arc<GitWorkflow>,
}

#[derive(Debug)]
pub struct TaskResult {
    pub task_id: String,
    pub success: bool,
    pub output: String,
    pub commits: Vec<String>,
}

impl Orchestrator {
    pub fn new(
        tasks: Vec<Task>,
        max_parallel: usize,
        event_sink: Arc<dyn EventSink>,
        git_workflow: Arc<GitWorkflow>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            scheduler: TaskScheduler::new(tasks)?,
            max_parallel,
            event_sink,
            git_workflow,
        })
    }

    pub async fn run(
        &mut self,
        conversation_loop: &ConversationLoop,
    ) -> Vec<TaskResult> {
        let mut results = Vec::new();
        let mut active: Vec<tokio::task::JoinHandle<TaskResult>> = Vec::new();

        loop {
            // Spawn ready tasks up to max_parallel
            while active.len() < self.max_parallel {
                let ready = self.scheduler.ready_tasks();
                if ready.is_empty() {
                    break;
                }
                let task = ready[0].clone();
                self.scheduler.start(&task.id);

                self.event_sink.emit(RuntimeEvent::TurnStart {
                    task: format!("Task {}: {}", task.id, task.description),
                });

                // Spawn sub-agent for this task
                let handle = self.spawn_task_agent(conversation_loop, task).await;
                active.push(handle);
            }

            if active.is_empty() {
                break; // All done or deadlocked
            }

            // Wait for any task to complete
            let (result, _index, remaining) = futures::future::select_all(active).await;
            active = remaining;

            match result {
                Ok(task_result) => {
                    self.scheduler.complete(&task_result.task_id);
                    self.event_sink.emit(RuntimeEvent::TurnEnd {
                        iterations: 1,
                        total_usage: Default::default(),
                    });
                    results.push(task_result);
                }
                Err(e) => {
                    tracing::error!("task agent panicked: {e}");
                }
            }
        }

        results
    }

    async fn spawn_task_agent(
        &self,
        parent: &ConversationLoop,
        task: Task,
    ) -> tokio::task::JoinHandle<TaskResult> {
        let event_sink = self.event_sink.clone();
        let task_id = task.id.clone();

        let prompt = format!(
            "Implement task {}: {}\n\nRelevant files: {}",
            task.id, task.description, task.files.join(", ")
        );

        // Spawn sub-agent using the runtime's agent spawning mechanism
        let sub_agent_handle = parent.spawn_sub_agent(runtime::tools::AgentRequest {
            task: prompt.clone(),
            tools: None, // inherit parent tools
            max_iterations: Some(25),
            files: Some(task.files.clone()),
        });

        tokio::spawn(async move {
            event_sink.emit(RuntimeEvent::TurnStart { task: prompt });

            let (success, output) = match sub_agent_handle.await {
                Ok(text) => (true, text),
                Err(e) => (false, format!("task failed: {e}")),
            };

            TaskResult {
                task_id,
                success,
                output,
                commits: vec![],
            }
        })
    }
}
```

Add `futures = "0.3"` to Cargo.toml.

- [ ] **Step 2: Wire orchestrator into drone execute phase**

In `src/drones/native/src/drone.rs`, for Implement stage:

```rust
Stage::Implement => {
    // Read plan file
    let plan_path = job.config.get("plan_path").expect("implement stage requires plan_path");
    let plan_content = tokio::fs::read_to_string(workspace.join(plan_path)).await?;
    let tasks = orchestrator::parse_plan(&plan_content);

    let mut orch = Orchestrator::new(tasks, 2, event_bridge.clone(), git_workflow.clone())?;
    let results = orch.run(&conversation).await;

    // Run full test suite after all tasks
    let test_result = runtime::tools::test_runner::run_tests("cargo test", &workspace).await;
    if test_result.failed > 0 {
        // Single-agent fix-up loop
        conversation.run_turn(&format!("Tests are failing after implementation. Fix these failures:\n{}", test_result.format_failures())).await?;
    }
}
```

- [ ] **Step 3: Run tests, buckify, verify build**

Run: `cd src/drones/native && cargo test`
Run: `./tools/buckify.sh`
Run: `buck2 build root//src/drones/native:native-drone`

- [ ] **Step 4: Commit**

```bash
git add src/drones/native/ Cargo.lock third-party/BUCK
git commit -m "add orchestrator with parallel task execution and git serialization"
```
