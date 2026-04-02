use crate::tool::{Tool, ToolSpec};
use std::collections::HashMap;
use std::sync::Arc;

/// Central registry of all available tools for a session.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    /// Known-but-not-loaded tool schemas for deferred/on-demand discovery.
    catalog: HashMap<String, ToolSpec>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            catalog: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.spec().name.clone();
        self.tools.insert(name, tool);
    }

    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn schemas(&self) -> Vec<&ToolSpec> {
        self.tools.values().map(|t| t.spec()).collect()
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Register a tool schema in the catalog (known but not loaded).
    /// Used for deferred discovery: the tool can be loaded on demand.
    pub fn register_catalog_entry(&mut self, spec: ToolSpec) {
        self.catalog.insert(spec.name.clone(), spec);
    }

    /// Search the catalog by keyword.
    pub fn search_catalog(&self, query: &str) -> Vec<&ToolSpec> {
        let q = query.to_lowercase();
        self.catalog
            .values()
            .filter(|s| {
                s.name.to_lowercase().contains(&q)
                    || s.description.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Promote a tool from runtime registration (e.g., after MCP on-demand connect).
    pub fn register_dynamic(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.spec().name.clone();
        self.catalog.remove(&name);
        self.tools.insert(name, tool);
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::*;
    use async_trait::async_trait;
    use serde_json::Value;

    struct FakeTool {
        spec: ToolSpec,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn spec(&self) -> &ToolSpec {
            &self.spec
        }
        fn validate(&self, _input: &Value) -> Result<(), ToolError> {
            Ok(())
        }
        async fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(Value::Null))
        }
    }

    fn fake(name: &str) -> Arc<dyn Tool> {
        Arc::new(FakeTool {
            spec: ToolSpec {
                name: name.into(),
                description: format!("{name} tool"),
                input_schema: serde_json::json!({}),
                concurrency: ConcurrencyClass::Concurrent,
            },
        })
    }

    #[test]
    fn register_and_get() {
        let mut reg = ToolRegistry::new();
        reg.register(fake("Read"));
        reg.register(fake("Write"));

        assert_eq!(reg.len(), 2);
        assert!(reg.get("Read").is_some());
        assert!(reg.get("Missing").is_none());
    }

    #[test]
    fn unregister() {
        let mut reg = ToolRegistry::new();
        reg.register(fake("Read"));
        let removed = reg.unregister("Read");
        assert!(removed.is_some());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn catalog_search() {
        let mut reg = ToolRegistry::new();
        reg.register_catalog_entry(ToolSpec {
            name: "mcp__github__issues".into(),
            description: "List GitHub issues".into(),
            input_schema: serde_json::json!({}),
            concurrency: ConcurrencyClass::Concurrent,
        });

        let results = reg.search_catalog("github");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "mcp__github__issues");

        let results = reg.search_catalog("nonexistent");
        assert!(results.is_empty());
    }
}
