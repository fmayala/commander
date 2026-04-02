use commander_tools::path_guard::{BoundaryViolation, PathGuard};
use std::path::Path;

/// Layer 2 implementation of PathGuard.
/// Checks file paths against the task's allowed file list and profile scope.
pub struct TaskBoundaryGuard {
    /// Allowed path patterns (glob). If empty, all paths are allowed.
    allowed_patterns: Vec<glob::Pattern>,
}

impl TaskBoundaryGuard {
    pub fn new(patterns: Vec<String>) -> Self {
        let allowed_patterns = patterns
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();
        Self { allowed_patterns }
    }

    /// Create a guard that allows all paths (no restrictions).
    pub fn allow_all() -> Self {
        Self {
            allowed_patterns: Vec::new(),
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
}
