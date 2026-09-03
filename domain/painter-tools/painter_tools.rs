//! Painter tools encapsulation root: one folder per tool under
//! `individuals/`, one registry seam in `registry.rs`.

#[path = "shared/selection_rules.rs"]
pub mod shared;

#[path = "registry.rs"]
pub mod registry;

#[path = "individuals/brush/brush_tool.rs"]
pub mod brush;

#[path = "individuals/erase/erase_tool.rs"]
pub mod erase;

#[path = "individuals/fill/fill_tool.rs"]
pub mod fill;

#[path = "individuals/lasso/lasso_tool.rs"]
pub mod lasso;

#[path = "individuals/text/text_tool.rs"]
pub mod text;

pub use registry::{all, by_id, require_by_id, RegisteredTool, ToolDescriptor};
