use serde::{Deserialize, Serialize};

/// What a rule does when it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleEffect {
    Allow,
    Deny,
}

/// A single permission rule: "if tool name matches this pattern, apply this effect."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    /// Glob pattern matched against the tool name.
    /// Examples: "Bash", "Bash(rm *)", "mcp__*", "Read"
    pub tool_pattern: String,
    /// Allow or Deny.
    pub effect: RuleEffect,
    /// Human-readable reason (shown to user on deny).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PermissionRule {
    pub fn deny(pattern: impl Into<String>) -> Self {
        Self {
            tool_pattern: pattern.into(),
            effect: RuleEffect::Deny,
            reason: None,
        }
    }

    pub fn allow(pattern: impl Into<String>) -> Self {
        Self {
            tool_pattern: pattern.into(),
            effect: RuleEffect::Allow,
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Does this rule's pattern match the given tool name?
    pub fn matches(&self, tool_name: &str) -> bool {
        let pat = glob::Pattern::new(&self.tool_pattern);
        match pat {
            Ok(p) => p.matches(tool_name),
            Err(_) => self.tool_pattern == tool_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        let rule = PermissionRule::deny("Bash");
        assert!(rule.matches("Bash"));
        assert!(!rule.matches("Read"));
    }

    #[test]
    fn glob_star() {
        let rule = PermissionRule::deny("mcp__*");
        assert!(rule.matches("mcp__github__create_issue"));
        assert!(!rule.matches("Read"));
    }

    #[test]
    fn glob_question() {
        let rule = PermissionRule::allow("Bash?rm*");
        // glob ? matches exactly one char, so "Bash(rm*)" won't match with ?
        // This is fine; patterns like "Bash(rm *)" use parens literally
        assert!(!rule.matches("Bash"));
    }
}
