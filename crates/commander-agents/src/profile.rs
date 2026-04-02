use commander_permissions::PermissionMode;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Agent profile: YAML frontmatter + markdown body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub name: String,
    #[serde(default = "default_model")]
    pub model: String,
    /// None = all tools available.
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_timeout")]
    pub timeout: String,

    /// The markdown body (scope, expertise, decision protocol).
    /// Not part of YAML frontmatter; parsed separately.
    #[serde(skip)]
    pub system_prompt: String,

    /// Path to the profile file on disk.
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

fn default_model() -> String {
    "claude-opus-4-6".into()
}

fn default_max_turns() -> u32 {
    50
}

fn default_timeout() -> String {
    "30m".into()
}

impl AgentProfile {
    /// Parse a markdown file with YAML frontmatter into an AgentProfile.
    pub fn from_markdown(content: &str, source_path: Option<&Path>) -> Result<Self, ProfileError> {
        let (frontmatter, body) = split_frontmatter(content)?;
        let mut profile: AgentProfile = serde_json::from_value(
            serde_json::to_value(
                serde_yaml_ng::from_str::<serde_json::Value>(&frontmatter)
                    .map_err(|e| ProfileError::YamlParse(e.to_string()))?,
            )
            .map_err(|e| ProfileError::YamlParse(e.to_string()))?,
        )
        .map_err(|e| ProfileError::YamlParse(e.to_string()))?;

        profile.system_prompt = body.trim().to_string();
        profile.source_path = source_path.map(|p| p.to_path_buf());
        Ok(profile)
    }

    /// Load a profile from a file path.
    pub fn from_file(path: &Path) -> Result<Self, ProfileError> {
        let content = std::fs::read_to_string(path).map_err(|e| ProfileError::Io(e.to_string()))?;
        Self::from_markdown(&content, Some(path))
    }
}

fn split_frontmatter(content: &str) -> Result<(String, String), ProfileError> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(ProfileError::MissingFrontmatter);
    }

    let after_first = &trimmed[3..];
    let end = after_first
        .find("\n---")
        .ok_or(ProfileError::MissingFrontmatter)?;

    let frontmatter = after_first[..end].trim().to_string();
    let body = after_first[end + 4..].to_string();
    Ok((frontmatter, body))
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("missing YAML frontmatter (expected --- delimiters)")]
    MissingFrontmatter,
    #[error("YAML parse error: {0}")]
    YamlParse(String),
    #[error("io error: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_profile() {
        let md = r#"---
name: "default-worker"
model: "claude-opus-4-6"
permission_mode: "auto"
max_turns: 50
timeout: "30m"
---

# System prompt
You are a software engineer working on the project.

## Scope
- src/**/*.rs
"#;
        let profile = AgentProfile::from_markdown(md, None).unwrap();
        assert_eq!(profile.name, "default-worker");
        assert_eq!(profile.model, "claude-opus-4-6");
        assert_eq!(profile.permission_mode, PermissionMode::AutoApprove);
        assert_eq!(profile.max_turns, 50);
        assert!(profile.system_prompt.contains("software engineer"));
        assert!(profile.tools.is_none());
    }

    #[test]
    fn parse_with_tools_list() {
        let md = r#"---
name: "read-only"
tools:
  - Read
  - Glob
  - Grep
permission_mode: "ask"
---

Read-only agent.
"#;
        let profile = AgentProfile::from_markdown(md, None).unwrap();
        assert_eq!(
            profile.tools,
            Some(vec!["Read".into(), "Glob".into(), "Grep".into()])
        );
        assert_eq!(profile.permission_mode, PermissionMode::Ask);
    }

    #[test]
    fn missing_frontmatter_errors() {
        let md = "no frontmatter here";
        assert!(AgentProfile::from_markdown(md, None).is_err());
    }
}
