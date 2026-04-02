use commander_tasks::task::Task;

/// Evaluate key_expr against a Task to derive a group key.
///
/// Built-in expressions:
/// - "project_id" -> task.project_id
/// - "global" -> "_global" (single shared pool)
/// - "files[0].path" -> first file path (or "_no_files")
pub fn derive_key(task: &Task, key_expr: &str) -> String {
    match key_expr {
        "project_id" => task.project_id.clone(),
        "global" => "_global".into(),
        "files[0].path" => task
            .files
            .first()
            .map(|f| f.path.clone())
            .unwrap_or_else(|| "_no_files".into()),
        // Future: support arbitrary field paths
        _ => {
            tracing::warn!(key_expr, "unknown key_expr, falling back to global");
            "_global".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commander_tasks::task::{FileAction, FileActionKind};

    fn task_with_project(project: &str) -> Task {
        Task::new("t1", project, "test")
    }

    #[test]
    fn derive_project_id() {
        let task = task_with_project("myproject");
        assert_eq!(derive_key(&task, "project_id"), "myproject");
    }

    #[test]
    fn derive_global() {
        let task = task_with_project("anything");
        assert_eq!(derive_key(&task, "global"), "_global");
    }

    #[test]
    fn derive_first_file() {
        let mut task = task_with_project("p");
        task.files.push(FileAction {
            path: "src/main.rs".into(),
            action: FileActionKind::Modify,
        });
        assert_eq!(derive_key(&task, "files[0].path"), "src/main.rs");
    }

    #[test]
    fn derive_first_file_empty() {
        let task = task_with_project("p");
        assert_eq!(derive_key(&task, "files[0].path"), "_no_files");
    }
}
