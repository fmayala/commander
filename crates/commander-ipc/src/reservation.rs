use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Exclusive file lock preventing concurrent writes by different agents.
///
/// IMPORTANT: This is NOT the same as boundary scope (which paths an agent is
/// allowed to modify). Reservations are mutual-exclusion locks between agents.
/// Scope enforcement is done by PathGuard in commander-tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReservation {
    pub path_pattern: String,
    pub holder: String,
    pub expires_at: DateTime<Utc>,
    pub exclusive: bool,
}

#[derive(Debug, Error)]
pub enum ReservationError {
    #[error("path {path} already reserved by {holder}")]
    Conflict { path: String, holder: String },
    #[error("reservation not found")]
    NotFound,
}

/// In-memory reservation manager for v0.
pub struct ReservationManager {
    reservations: HashMap<String, FileReservation>,
}

impl ReservationManager {
    pub fn new() -> Self {
        Self {
            reservations: HashMap::new(),
        }
    }

    /// Try to reserve a path pattern for an agent.
    pub fn reserve(
        &mut self,
        pattern: &str,
        holder: &str,
        duration: chrono::Duration,
    ) -> Result<(), ReservationError> {
        // Check for conflicts with existing reservations
        for (_, existing) in &self.reservations {
            if existing.exclusive && patterns_overlap(pattern, &existing.path_pattern) {
                if existing.holder != holder && existing.expires_at > Utc::now() {
                    return Err(ReservationError::Conflict {
                        path: pattern.into(),
                        holder: existing.holder.clone(),
                    });
                }
            }
        }

        self.reservations.insert(
            format!("{}:{}", holder, pattern),
            FileReservation {
                path_pattern: pattern.into(),
                holder: holder.into(),
                expires_at: Utc::now() + duration,
                exclusive: true,
            },
        );
        Ok(())
    }

    /// Release a reservation.
    pub fn release(&mut self, pattern: &str, holder: &str) -> Result<(), ReservationError> {
        let key = format!("{holder}:{pattern}");
        self.reservations
            .remove(&key)
            .ok_or(ReservationError::NotFound)?;
        Ok(())
    }

    /// Check if a path is reserved by someone other than the given agent.
    pub fn is_reserved_by_other(&self, path: &str, agent_id: &str) -> Option<&str> {
        for res in self.reservations.values() {
            if res.holder != agent_id
                && res.exclusive
                && res.expires_at > Utc::now()
                && path_matches_pattern(path, &res.path_pattern)
            {
                return Some(&res.holder);
            }
        }
        None
    }

    /// Remove expired reservations.
    pub fn gc(&mut self) {
        let now = Utc::now();
        self.reservations.retain(|_, r| r.expires_at > now);
    }
}

impl Default for ReservationManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple glob-like overlap check: two patterns overlap if either could match
/// paths the other covers. This is conservative (may report false overlaps).
fn patterns_overlap(a: &str, b: &str) -> bool {
    // If either contains a wildcard, check prefix overlap
    let a_base = a.split('*').next().unwrap_or(a);
    let b_base = b.split('*').next().unwrap_or(b);
    a_base.starts_with(b_base) || b_base.starts_with(a_base)
}

fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    glob::Pattern::new(pattern)
        .map(|p| p.matches(path))
        .unwrap_or(path == pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_and_check() {
        let mut mgr = ReservationManager::new();
        mgr.reserve("src/auth/**", "agent-1", chrono::Duration::minutes(30))
            .unwrap();

        assert!(mgr
            .is_reserved_by_other("src/auth/login.rs", "agent-2")
            .is_some());
        assert!(mgr
            .is_reserved_by_other("src/auth/login.rs", "agent-1")
            .is_none());
        assert!(mgr
            .is_reserved_by_other("src/other/file.rs", "agent-2")
            .is_none());
    }

    #[test]
    fn conflict_on_overlap() {
        let mut mgr = ReservationManager::new();
        mgr.reserve("src/auth/**", "agent-1", chrono::Duration::minutes(30))
            .unwrap();

        let err = mgr
            .reserve("src/auth/**", "agent-2", chrono::Duration::minutes(30))
            .unwrap_err();
        assert!(matches!(err, ReservationError::Conflict { .. }));
    }

    #[test]
    fn same_holder_can_re_reserve() {
        let mut mgr = ReservationManager::new();
        mgr.reserve("src/**", "agent-1", chrono::Duration::minutes(30))
            .unwrap();
        // Same holder can extend
        mgr.reserve("src/**", "agent-1", chrono::Duration::minutes(60))
            .unwrap();
    }

    #[test]
    fn release_allows_new_holder() {
        let mut mgr = ReservationManager::new();
        mgr.reserve("src/auth/**", "agent-1", chrono::Duration::minutes(30))
            .unwrap();
        mgr.release("src/auth/**", "agent-1").unwrap();

        mgr.reserve("src/auth/**", "agent-2", chrono::Duration::minutes(30))
            .unwrap();
    }
}
