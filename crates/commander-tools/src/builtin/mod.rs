pub mod read;
pub mod write;
pub mod bash;

pub use read::ReadTool;
pub use write::WriteTool;
pub use bash::BashTool;

use crate::registry::ToolRegistry;
use std::sync::Arc;

/// Register all built-in tools in the registry.
pub fn register_builtins(registry: &mut ToolRegistry) {
    registry.register(Arc::new(ReadTool));
    registry.register(Arc::new(WriteTool));
    registry.register(Arc::new(BashTool));
}
