use crate::message::Message;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Error)]
pub enum TranscriptError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error on line {line}: {source}")]
    Json {
        line: usize,
        source: serde_json::Error,
    },
}

/// Append-only JSONL writer. One line per message.
pub struct TranscriptWriter {
    path: PathBuf,
    file: File,
}

impl TranscriptWriter {
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self, TranscriptError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        Ok(Self { path, file })
    }

    pub async fn append(&mut self, msg: &Message) -> Result<(), TranscriptError> {
        let mut line =
            serde_json::to_string(msg).map_err(|e| TranscriptError::Json { line: 0, source: e })?;
        line.push('\n');
        self.file.write_all(line.as_bytes()).await?;
        self.file.flush().await?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Reads a JSONL transcript back into a message list.
pub struct TranscriptReader {
    path: PathBuf,
}

impl TranscriptReader {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub async fn load(&self) -> Result<Vec<Message>, TranscriptError> {
        let file = File::open(&self.path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut messages = Vec::new();
        let mut line_num = 0usize;

        while let Some(line) = lines.next_line().await? {
            line_num += 1;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let msg: Message =
                serde_json::from_str(trimmed).map_err(|e| TranscriptError::Json {
                    line: line_num,
                    source: e,
                })?;
            messages.push(msg);
        }

        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;
    use tempfile::TempDir;

    #[tokio::test]
    async fn roundtrip_write_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.jsonl");

        // Write 3 messages
        let mut writer = TranscriptWriter::open(&path).await.unwrap();
        writer.append(&Message::user("hello")).await.unwrap();
        writer
            .append(&Message::assistant("hi there"))
            .await
            .unwrap();
        writer
            .append(&Message::system("you are a helpful assistant"))
            .await
            .unwrap();
        drop(writer);

        // Read them back
        let reader = TranscriptReader::new(&path);
        let msgs = reader.load().await.unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].text(), Some("hello"));
        assert_eq!(msgs[1].text(), Some("hi there"));
        assert_eq!(msgs[2].text(), Some("you are a helpful assistant"));
    }

    #[tokio::test]
    async fn append_to_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.jsonl");

        // Write one, close, reopen, write another
        let mut w1 = TranscriptWriter::open(&path).await.unwrap();
        w1.append(&Message::user("first")).await.unwrap();
        drop(w1);

        let mut w2 = TranscriptWriter::open(&path).await.unwrap();
        w2.append(&Message::user("second")).await.unwrap();
        drop(w2);

        let msgs = TranscriptReader::new(&path).load().await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text(), Some("first"));
        assert_eq!(msgs[1].text(), Some("second"));
    }

    #[tokio::test]
    async fn empty_lines_skipped() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.jsonl");

        let mut writer = TranscriptWriter::open(&path).await.unwrap();
        writer.append(&Message::user("only")).await.unwrap();
        drop(writer);

        // Manually append empty line
        tokio::fs::write(
            &path,
            format!(
                "{}\n\n",
                serde_json::to_string(&Message::user("only")).unwrap()
            ),
        )
        .await
        .unwrap();

        let msgs = TranscriptReader::new(&path).load().await.unwrap();
        assert_eq!(msgs.len(), 1);
    }
}
