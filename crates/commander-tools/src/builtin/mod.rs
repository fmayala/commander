pub mod bash;
pub mod complete_task;
pub mod read;
pub mod write;

pub use bash::BashTool;
pub use complete_task::CompleteTaskTool;
pub use read::ReadTool;
pub use write::WriteTool;

use crate::registry::ToolRegistry;
use std::sync::Arc;

/// Register all built-in tools in the registry.
pub fn register_builtins(registry: &mut ToolRegistry) {
    registry.register(Arc::new(ReadTool));
    registry.register(Arc::new(WriteTool));
    registry.register(Arc::new(BashTool));
    registry.register(Arc::new(CompleteTaskTool));
}
