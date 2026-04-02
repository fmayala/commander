use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("boundary violation: {path} is outside allowed scope")]
pub struct BoundaryViolation {
    pub path: String,
}

/// Checks whether a path is within the agent's allowed scope.
///
/// Defined in Layer 1 (commander-tools). Implemented by Layer 2 (commander-coordination)
/// as `TaskBoundaryGuard`. In interactive/standalone sessions, no guard is set and
/// all paths are allowed.
pub trait PathGuard: Send + Sync {
    fn check_write(&self, path: &Path) -> Result<(), BoundaryViolation>;

    /// Check if a path is within the allowed read scope.
    /// Defaults to the same rules as write access.
    fn check_read(&self, path: &Path) -> Result<(), BoundaryViolation> {
        self.check_write(path)
    }
}
