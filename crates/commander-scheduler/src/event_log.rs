use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EventLogError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error on line {line}: {source}")]
    Json {
        line: usize,
        source: serde_json::Error,
    },
}

/// A single event record in the NDJSON log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub run_id: String,
    pub agent_id: String,
    pub kind: EventKind,
    pub timestamp: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    RunCreated,
    StateChanged,
    ToolCallStarted,
    ToolCallCompleted,
    CheckpointSaved,
    WaitStarted,
    WaitCompleted,
    Completed,
    Failed,
}

/// Append-only NDJSON event log. One file per agent run.
///
/// On open, performs torn-line recovery: scans backward from EOF,
/// truncates any partial final line, logs a warning.
pub struct EventLog {
    path: PathBuf,
    file: std::fs::File,
}

impl EventLog {
    /// Open (or create) an event log file with torn-line recovery.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, EventLogError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // If the file exists, do torn-line recovery
        if path.exists() {
            recover_torn_line(&path)?;
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        Ok(Self { path, file })
    }

    pub fn append(&mut self, record: &EventRecord) -> Result<(), EventLogError> {
        let mut line = serde_json::to_string(record)
            .map_err(|e| EventLogError::Json { line: 0, source: e })?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read all records from the log.
    pub fn replay(path: &Path) -> Result<Vec<EventRecord>, EventLogError> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let mut records = Vec::new();

        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let record: EventRecord =
                serde_json::from_str(trimmed).map_err(|e| EventLogError::Json {
                    line: i + 1,
                    source: e,
                })?;
            records.push(record);
        }

        Ok(records)
    }
}

/// Torn-line recovery: find last complete newline-terminated record,
/// truncate anything after it.
fn recover_torn_line(path: &Path) -> Result<(), EventLogError> {
    let content = std::fs::read(path)?;
    if content.is_empty() {
        return Ok(());
    }

    // Find position of last newline
    let last_newline = content.iter().rposition(|&b| b == b'\n');
    let valid_len = match last_newline {
        Some(pos) => pos + 1, // include the newline
        None => 0,            // no complete line at all
    };

    if valid_len < content.len() {
        let discarded = content.len() - valid_len;
        tracing::warn!(
            path = %path.display(),
            discarded_bytes = discarded,
            "event log torn-line recovery: discarded {discarded} bytes after last newline"
        );
        let file = std::fs::OpenOptions::new().write(true).open(path)?;
        file.set_len(valid_len as u64)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_record(kind: EventKind) -> EventRecord {
        EventRecord {
            run_id: "run-1".into(),
            agent_id: "agent-1".into(),
            kind,
            timestamp: Utc::now(),
            payload: Value::Null,
        }
    }

    #[test]
    fn write_and_replay() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("events.ndjson");

        let mut log = EventLog::open(&path).unwrap();
        log.append(&make_record(EventKind::RunCreated)).unwrap();
        log.append(&make_record(EventKind::ToolCallStarted))
            .unwrap();
        log.append(&make_record(EventKind::Completed)).unwrap();
        drop(log);

        let records = EventLog::replay(&path).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].kind, EventKind::RunCreated);
        assert_eq!(records[2].kind, EventKind::Completed);
    }

    #[test]
    fn torn_line_recovery() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("events.ndjson");

        // Write 2 complete records + partial junk
        let mut log = EventLog::open(&path).unwrap();
        log.append(&make_record(EventKind::RunCreated)).unwrap();
        log.append(&make_record(EventKind::ToolCallStarted))
            .unwrap();
        drop(log);

        // Append partial junk (simulate crash mid-write)
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"{\"partial\":true").unwrap();
        drop(file);

        // Reopen — should recover cleanly
        let log2 = EventLog::open(&path).unwrap();
        drop(log2);

        let records = EventLog::replay(&path).unwrap();
        assert_eq!(records.len(), 2); // partial line discarded
    }

    #[test]
    fn empty_file_ok() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.ndjson");

        std::fs::write(&path, "").unwrap();
        let _log = EventLog::open(&path).unwrap();
        let records = EventLog::replay(&path).unwrap();
        assert!(records.is_empty());
    }
}
