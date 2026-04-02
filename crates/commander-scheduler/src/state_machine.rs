use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Layer 3 run status (maps to Layer 2 TaskStatus but with scheduling semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Suspended,
    Retrying,
    Done,
    Failed,
    Escalated,
}

#[derive(Debug, Clone, Copy)]
pub enum TransitionCommand {
    Assign,
    Suspend,
    Resume,
    Complete,
    Retry,
    Fail,
    Escalate,
}

#[derive(Debug, Error)]
#[error("invalid transition: {from:?} + {command:?}")]
pub struct TransitionError {
    pub from: RunStatus,
    pub command: TransitionCommand,
}

pub fn transition(
    current: RunStatus,
    command: TransitionCommand,
) -> Result<RunStatus, TransitionError> {
    match (current, command) {
        (RunStatus::Pending, TransitionCommand::Assign) => Ok(RunStatus::Running),
        (RunStatus::Running, TransitionCommand::Suspend) => Ok(RunStatus::Suspended),
        (RunStatus::Running, TransitionCommand::Complete) => Ok(RunStatus::Done),
        (RunStatus::Running, TransitionCommand::Retry) => Ok(RunStatus::Retrying),
        (RunStatus::Running, TransitionCommand::Fail) => Ok(RunStatus::Failed),
        (RunStatus::Suspended, TransitionCommand::Resume) => Ok(RunStatus::Pending),
        (RunStatus::Retrying, TransitionCommand::Assign) => Ok(RunStatus::Running),
        (_, TransitionCommand::Escalate) => Ok(RunStatus::Escalated),
        _ => Err(TransitionError {
            from: current,
            command,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path() {
        let s = transition(RunStatus::Pending, TransitionCommand::Assign).unwrap();
        assert_eq!(s, RunStatus::Running);
        let s = transition(s, TransitionCommand::Complete).unwrap();
        assert_eq!(s, RunStatus::Done);
    }

    #[test]
    fn retry_path() {
        let s = transition(RunStatus::Pending, TransitionCommand::Assign).unwrap();
        let s = transition(s, TransitionCommand::Retry).unwrap();
        assert_eq!(s, RunStatus::Retrying);
        let s = transition(s, TransitionCommand::Assign).unwrap();
        assert_eq!(s, RunStatus::Running);
    }

    #[test]
    fn suspend_resume() {
        let s = transition(RunStatus::Pending, TransitionCommand::Assign).unwrap();
        let s = transition(s, TransitionCommand::Suspend).unwrap();
        assert_eq!(s, RunStatus::Suspended);
        let s = transition(s, TransitionCommand::Resume).unwrap();
        assert_eq!(s, RunStatus::Pending);
    }

    #[test]
    fn escalate_from_any() {
        for status in [
            RunStatus::Pending,
            RunStatus::Running,
            RunStatus::Suspended,
            RunStatus::Retrying,
            RunStatus::Failed,
        ] {
            let s = transition(status, TransitionCommand::Escalate).unwrap();
            assert_eq!(s, RunStatus::Escalated);
        }
    }

    #[test]
    fn invalid_transition() {
        let err = transition(RunStatus::Done, TransitionCommand::Assign);
        assert!(err.is_err());
    }
}
