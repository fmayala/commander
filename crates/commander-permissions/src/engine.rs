use crate::mode::PermissionMode;
use crate::rule::PermissionRule;

/// Result of a permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny(String),
    Ask(String),
}

/// Stateless rule-based permission engine.
///
/// Decision priority: deny rules > allow rules > mode default.
pub struct PermissionEngine {
    pub mode: PermissionMode,
    pub deny_rules: Vec<PermissionRule>,
    pub allow_rules: Vec<PermissionRule>,
}

impl PermissionEngine {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            deny_rules: Vec::new(),
            allow_rules: Vec::new(),
        }
    }

    /// Check whether a tool call should be allowed, denied, or needs user approval.
    pub fn check(&self, tool_name: &str) -> PermissionDecision {
        // 1. Deny rules take highest priority
        for rule in &self.deny_rules {
            if rule.matches(tool_name) {
                let reason = rule
                    .reason
                    .clone()
                    .unwrap_or_else(|| format!("denied by rule: {}", rule.tool_pattern));
                return PermissionDecision::Deny(reason);
            }
        }

        // 2. Allow rules override mode default
        for rule in &self.allow_rules {
            if rule.matches(tool_name) {
                return PermissionDecision::Allow;
            }
        }

        // 3. Mode default
        match self.mode {
            PermissionMode::AutoApprove => PermissionDecision::Allow,
            PermissionMode::Normal => {
                // Normal: read-only tools are auto-allowed; writes ask.
                // For now, without knowing the tool's read-only flag, we ask.
                // The runtime will provide is_read_only context.
                PermissionDecision::Ask(format!("{tool_name} requires approval"))
            }
            PermissionMode::Ask => {
                PermissionDecision::Ask(format!("{tool_name} requires approval"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::PermissionRule;

    #[test]
    fn deny_rule_overrides_auto_approve() {
        let mut engine = PermissionEngine::new(PermissionMode::AutoApprove);
        engine
            .deny_rules
            .push(PermissionRule::deny("Bash").with_reason("no shell access"));

        assert_eq!(
            engine.check("Bash"),
            PermissionDecision::Deny("no shell access".into())
        );
        // Other tools pass through
        assert_eq!(engine.check("Read"), PermissionDecision::Allow);
    }

    #[test]
    fn allow_rule_overrides_ask_mode() {
        let mut engine = PermissionEngine::new(PermissionMode::Ask);
        engine.allow_rules.push(PermissionRule::allow("Read"));

        assert_eq!(engine.check("Read"), PermissionDecision::Allow);
        assert!(matches!(engine.check("Write"), PermissionDecision::Ask(_)));
    }

    #[test]
    fn auto_approve_allows_all() {
        let engine = PermissionEngine::new(PermissionMode::AutoApprove);
        assert_eq!(engine.check("Bash"), PermissionDecision::Allow);
        assert_eq!(engine.check("Write"), PermissionDecision::Allow);
    }

    #[test]
    fn glob_deny_mcp_tools() {
        let mut engine = PermissionEngine::new(PermissionMode::AutoApprove);
        engine.deny_rules.push(PermissionRule::deny("mcp__*"));

        assert!(matches!(
            engine.check("mcp__github__create_issue"),
            PermissionDecision::Deny(_)
        ));
        assert_eq!(engine.check("Read"), PermissionDecision::Allow);
    }

    #[test]
    fn deny_before_allow() {
        let mut engine = PermissionEngine::new(PermissionMode::Ask);
        engine.deny_rules.push(PermissionRule::deny("Bash"));
        engine.allow_rules.push(PermissionRule::allow("Bash"));

        // Deny wins even though allow also matches
        assert!(matches!(
            engine.check("Bash"),
            PermissionDecision::Deny(_)
        ));
    }

    #[test]
    fn serde_mode_alias() {
        let mode: PermissionMode = serde_json::from_str(r#""auto""#).unwrap();
        assert_eq!(mode, PermissionMode::AutoApprove);

        let mode: PermissionMode = serde_json::from_str(r#""AutoApprove""#).unwrap();
        assert_eq!(mode, PermissionMode::AutoApprove);

        let mode: PermissionMode = serde_json::from_str(r#""ask""#).unwrap();
        assert_eq!(mode, PermissionMode::Ask);
    }
}
