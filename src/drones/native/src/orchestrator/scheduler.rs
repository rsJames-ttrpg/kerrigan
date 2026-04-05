use std::collections::{HashMap, HashSet, VecDeque};

use super::plan_parser::Task;

pub struct TaskScheduler {
    tasks: Vec<Task>,
    completed: HashSet<String>,
    in_progress: HashSet<String>,
}

impl TaskScheduler {
    /// Create a new scheduler, validating that the task graph is a valid DAG.
    pub fn new(tasks: Vec<Task>) -> anyhow::Result<Self> {
        validate_dag(&tasks)?;
        Ok(Self {
            tasks,
            completed: HashSet::new(),
            in_progress: HashSet::new(),
        })
    }

    /// Get tasks whose dependencies are all satisfied and not yet started.
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

    /// Mark a task as in-progress.
    pub fn start(&mut self, task_id: &str) {
        self.in_progress.insert(task_id.to_string());
    }

    /// Mark a task as completed.
    pub fn complete(&mut self, task_id: &str) {
        self.in_progress.remove(task_id);
        self.completed.insert(task_id.to_string());
    }

    /// Returns true when all tasks have been completed.
    pub fn is_done(&self) -> bool {
        self.completed.len() == self.tasks.len()
    }

    /// Number of tasks not yet completed.
    pub fn remaining(&self) -> usize {
        self.tasks.len() - self.completed.len()
    }
}

/// Validate that the task graph forms a valid DAG: all dependencies exist and
/// there are no cycles.
fn validate_dag(tasks: &[Task]) -> anyhow::Result<()> {
    let ids: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();

    // Check all dependencies reference existing tasks.
    for task in tasks {
        for dep in &task.dependencies {
            anyhow::ensure!(
                ids.contains(dep.as_str()),
                "unknown dependency: {dep} in task {}",
                task.id
            );
        }
    }

    // Cycle detection via Kahn's algorithm (topological sort).
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for task in tasks {
        in_degree.entry(task.id.as_str()).or_insert(0);
        for dep in &task.dependencies {
            *in_degree.entry(task.id.as_str()).or_insert(0) += 1;
            dependents
                .entry(dep.as_str())
                .or_default()
                .push(task.id.as_str());
        }
    }

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut visited = 0usize;

    while let Some(id) = queue.pop_front() {
        visited += 1;
        if let Some(deps) = dependents.get(id) {
            for &dep_id in deps {
                if let Some(deg) = in_degree.get_mut(dep_id) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dep_id);
                    }
                }
            }
        }
    }

    anyhow::ensure!(
        visited == tasks.len(),
        "cycle detected in task dependency graph"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, deps: &[&str]) -> Task {
        Task {
            id: id.to_string(),
            description: format!("Task {id}"),
            dependencies: deps.iter().map(|d| d.to_string()).collect(),
            files: vec![],
        }
    }

    #[test]
    fn test_ready_tasks_no_deps() {
        let scheduler = TaskScheduler::new(vec![task("a", &[]), task("b", &[])]).unwrap();
        let ready = scheduler.ready_tasks();
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn test_ready_tasks_after_completion() {
        let mut scheduler =
            TaskScheduler::new(vec![task("a", &[]), task("b", &["a"])]).unwrap();

        let ready = scheduler.ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "a");

        scheduler.start("a");
        assert!(scheduler.ready_tasks().is_empty());

        scheduler.complete("a");
        let ready = scheduler.ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "b");
    }

    #[test]
    fn test_cycle_detection() {
        let result =
            TaskScheduler::new(vec![task("a", &["b"]), task("b", &["a"])]);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("cycle detected")
        );
    }

    #[test]
    fn test_unknown_dependency() {
        let result = TaskScheduler::new(vec![task("a", &["nonexistent"])]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unknown dependency")
        );
    }

    #[test]
    fn test_is_done() {
        let mut scheduler = TaskScheduler::new(vec![task("a", &[])]).unwrap();
        assert!(!scheduler.is_done());
        scheduler.start("a");
        assert!(!scheduler.is_done());
        scheduler.complete("a");
        assert!(scheduler.is_done());
    }

    #[test]
    fn test_remaining() {
        let mut scheduler =
            TaskScheduler::new(vec![task("a", &[]), task("b", &["a"])]).unwrap();
        assert_eq!(scheduler.remaining(), 2);
        scheduler.start("a");
        scheduler.complete("a");
        assert_eq!(scheduler.remaining(), 1);
    }

    #[test]
    fn test_diamond_dependency() {
        // a -> b, a -> c, b -> d, c -> d
        let mut scheduler = TaskScheduler::new(vec![
            task("a", &[]),
            task("b", &["a"]),
            task("c", &["a"]),
            task("d", &["b", "c"]),
        ])
        .unwrap();

        let ready = scheduler.ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "a");

        scheduler.start("a");
        scheduler.complete("a");

        let ready = scheduler.ready_tasks();
        assert_eq!(ready.len(), 2);
        let ids: Vec<&str> = ready.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));

        scheduler.start("b");
        scheduler.complete("b");
        // d still waiting on c
        assert!(scheduler.ready_tasks().iter().all(|t| t.id != "d"));

        scheduler.start("c");
        scheduler.complete("c");
        let ready = scheduler.ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "d");
    }

    #[test]
    fn test_empty_task_list() {
        let scheduler = TaskScheduler::new(vec![]).unwrap();
        assert!(scheduler.is_done());
        assert!(scheduler.ready_tasks().is_empty());
    }

    #[test]
    fn test_three_node_cycle() {
        let result = TaskScheduler::new(vec![
            task("a", &["c"]),
            task("b", &["a"]),
            task("c", &["b"]),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_in_progress_not_ready() {
        let mut scheduler =
            TaskScheduler::new(vec![task("a", &[]), task("b", &[])]).unwrap();
        scheduler.start("a");
        let ready = scheduler.ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "b");
    }
}
