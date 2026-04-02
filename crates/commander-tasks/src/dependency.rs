use crate::task::{Task, TaskStatus};
use std::collections::{HashMap, HashSet};

/// Returns true if all dependencies of the given task are Complete.
pub fn deps_satisfied(task: &Task, tasks: &HashMap<String, Task>) -> bool {
    task.depends_on.iter().all(|dep_id| {
        tasks
            .get(dep_id)
            .map(|t| t.status == TaskStatus::Complete)
            .unwrap_or(false)
    })
}

/// Returns true if all subtasks of the given parent are Complete.
pub fn subtasks_complete(parent_id: &str, tasks: &HashMap<String, Task>) -> bool {
    tasks
        .values()
        .filter(|t| t.parent_id.as_deref() == Some(parent_id))
        .all(|t| t.status == TaskStatus::Complete)
}

/// Topological sort of task IDs respecting depends_on edges.
/// Returns None if there's a cycle.
pub fn topo_sort(tasks: &HashMap<String, Task>) -> Option<Vec<String>> {
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();

    for (id, task) in tasks {
        in_degree.entry(id.as_str()).or_insert(0);
        for dep in &task.depends_on {
            graph.entry(dep.as_str()).or_default().push(id.as_str());
            *in_degree.entry(id.as_str()).or_insert(0) += 1;
        }
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();
    queue.sort(); // deterministic order

    let mut result = Vec::new();
    let mut visited = HashSet::new();

    while let Some(id) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        result.push(id.to_string());
        if let Some(dependents) = graph.get(id) {
            for &dep in dependents {
                let deg = in_degree.get_mut(dep).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push(dep);
                }
            }
        }
    }

    if result.len() == tasks.len() {
        Some(result)
    } else {
        None // cycle
    }
}
