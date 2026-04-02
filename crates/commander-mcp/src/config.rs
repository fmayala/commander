use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransport,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout: Duration,
    #[serde(default = "default_request_timeout")]
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Sse {
        url: String,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

fn default_connect_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_request_timeout() -> Duration {
    Duration::from_secs(60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_stdio_config() {
        let json = serde_json::json!({
            "name": "github",
            "transport": {
                "type": "stdio",
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"]
            }
        });
        let config: McpServerConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.name, "github");
        match &config.transport {
            McpTransport::Stdio { command, args } => {
                assert_eq!(command, "npx");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("expected Stdio"),
        }
    }
}
