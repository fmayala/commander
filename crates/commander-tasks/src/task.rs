use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub priority: Priority,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub status: TaskStatus,
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    #[serde(default)]
    pub files: Vec<FileAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_artifact_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    P0,
    P1,
    P2,
    P3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Claimed,
    Blocked,
    Complete,
    Failed,
    Discovered,
    Retrying,
    Escalated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    Implement,
    Explore,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskKind::Implement => "implement",
            TaskKind::Explore => "explore",
        }
    }

    pub fn requires_in_scope_changes(self, has_allowed_scope: bool) -> bool {
        has_allowed_scope && matches!(self, TaskKind::Implement)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAction {
    pub path: String,
    pub action: FileActionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileActionKind {
    Create,
    Modify,
    Delete,
}

impl Task {
    pub fn new(
        id: impl Into<String>,
        project_id: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            project_id: project_id.into(),
            title: title.into(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            priority: Priority::P2,
            depends_on: Vec::new(),
            parent_id: None,
            status: TaskStatus::Pending,
            kind: TaskKind::Implement,
            claimed_by: None,
            files: Vec::new(),
            chain_artifact_id: None,
        }
    }

    pub fn with_priority(mut self, p: Priority) -> Self {
        self.priority = p;
        self
    }

    pub fn with_depends_on(mut self, deps: Vec<String>) -> Self {
        self.depends_on = deps;
        self
    }

    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    pub fn with_kind(mut self, kind: TaskKind) -> Self {
        self.kind = kind;
        self
    }
}
