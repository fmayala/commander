use commander_tools::path_guard::{BoundaryViolation, PathGuard};
use std::path::{Path, PathBuf};

/// Layer 2 implementation of PathGuard.
/// Checks file paths against the task's allowed file list and profile scope.
pub struct TaskBoundaryGuard {
    /// Allowed path patterns (glob). If empty, all paths are allowed.
    allowed_patterns: Vec<glob::Pattern>,
    /// Optional workspace root used to match relative task patterns
    /// (for example `webapp/src/App.jsx`) against absolute write paths.
    workspace_root: Option<PathBuf>,
}

impl TaskBoundaryGuard {
    pub fn new(patterns: Vec<String>) -> Self {
        let allowed_patterns = patterns
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();
        Self {
            allowed_patterns,
            workspace_root: None,
        }
    }

    pub fn new_with_workspace(patterns: Vec<String>, workspace_root: impl Into<PathBuf>) -> Self {
        let allowed_patterns = patterns
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();
        Self {
            allowed_patterns,
            workspace_root: Some(workspace_root.into()),
        }
    }

    /// Create a guard that allows all paths (no restrictions).
    pub fn allow_all() -> Self {
        Self {
            allowed_patterns: Vec::new(),
            workspace_root: None,
        }
    }
}

impl PathGuard for TaskBoundaryGuard {
    fn check_write(&self, path: &Path) -> Result<(), BoundaryViolation> {
        // If no patterns are configured, allow all (standalone/interactive mode).
        if self.allowed_patterns.is_empty() {
            return Ok(());
        }

        let path_str = path.to_string_lossy();
        for pattern in &self.allowed_patterns {
            if pattern.matches(&path_str) {
                return Ok(());
            }
        }

        if let Some(root) = &self.workspace_root {
            if let Ok(relative) = path.strip_prefix(root) {
                let rel_str = relative.to_string_lossy();
                for pattern in &self.allowed_patterns {
                    if pattern.matches(&rel_str) {
                        return Ok(());
                    }
                }
            }
        }

        Err(BoundaryViolation {
            path: path_str.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn allows_matching_path() {
        let guard = TaskBoundaryGuard::new(vec!["src/auth/**".into(), "tests/**".into()]);
        assert!(guard
            .check_write(&PathBuf::from("src/auth/login.rs"))
            .is_ok());
        assert!(guard
            .check_write(&PathBuf::from("tests/auth_test.rs"))
            .is_ok());
    }

    #[test]
    fn blocks_non_matching_path() {
        let guard = TaskBoundaryGuard::new(vec!["src/auth/**".into()]);
        let result = guard.check_write(&PathBuf::from("src/billing/charge.rs"));
        assert!(result.is_err());
    }

    #[test]
    fn allow_all_when_no_patterns() {
        let guard = TaskBoundaryGuard::allow_all();
        assert!(guard
            .check_write(&PathBuf::from("/literally/anything"))
            .is_ok());
    }

    #[test]
    fn empty_patterns_list_allows_all() {
        let guard = TaskBoundaryGuard::new(vec![]);
        assert!(guard.check_write(&PathBuf::from("any/path")).is_ok());
    }

    #[test]
    fn matches_relative_pattern_for_absolute_path_with_workspace_root() {
        let root = PathBuf::from("/tmp/project");
        let guard =
            TaskBoundaryGuard::new_with_workspace(vec!["webapp/src/App.jsx".into()], root.clone());
        assert!(guard.check_write(&root.join("webapp/src/App.jsx")).is_ok());
    }
}
