use crate::client::{McpClient, McpError};
use crate::config::McpServerConfig;
use crate::tool::McpTool;
use commander_tools::registry::ToolRegistry;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Manages multiple MCP server connections with on-demand connect.
pub struct McpManager {
    configs: HashMap<String, McpServerConfig>,
    clients: HashMap<String, Arc<Mutex<McpClient>>>,
}

impl McpManager {
    pub fn new(configs: Vec<McpServerConfig>) -> Self {
        let configs = configs
            .into_iter()
            .map(|c| (c.name.clone(), c))
            .collect();
        Self {
            configs,
            clients: HashMap::new(),
        }
    }

    /// Connect to a server on demand and return its client.
    pub async fn connect_on_demand(
        &mut self,
        server_name: &str,
    ) -> Result<Arc<Mutex<McpClient>>, McpError> {
        if let Some(client) = self.clients.get(server_name) {
            return Ok(Arc::clone(client));
        }

        let config = self
            .configs
            .get(server_name)
            .ok_or_else(|| McpError::ConnectionFailed(format!("unknown server: {server_name}")))?
            .clone();

        let client = McpClient::connect(config).await?;
        let client = Arc::new(Mutex::new(client));
        self.clients
            .insert(server_name.to_string(), Arc::clone(&client));
        Ok(client)
    }

    /// Connect to a server and register its tools in the provided registry.
    pub async fn discover_and_register(
        &mut self,
        server_name: &str,
        registry: &mut ToolRegistry,
    ) -> Result<usize, McpError> {
        let client = self.connect_on_demand(server_name).await?;
        let tools = {
            let mut c = client.lock().await;
            c.list_tools().await?
        };

        let count = tools.len();
        for info in tools {
            let mcp_tool = McpTool::new(
                server_name,
                &info.name,
                info.description.as_deref(),
                info.input_schema,
                Arc::clone(&client),
            );
            registry.register_dynamic(Arc::new(mcp_tool));
        }

        tracing::info!(server = server_name, tools = count, "registered MCP tools");
        Ok(count)
    }

    /// List all configured server names.
    pub fn server_names(&self) -> Vec<&str> {
        self.configs.keys().map(|s| s.as_str()).collect()
    }

    /// Check if a server is currently connected.
    pub fn is_connected(&self, server_name: &str) -> bool {
        self.clients.contains_key(server_name)
    }
}
