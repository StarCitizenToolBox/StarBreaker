//! BuildingBlocks canvas record parser and widget tree resolver.

mod parser;
mod resolver;
#[cfg(test)]
mod tests;
mod types;

pub use parser::CanvasParser;
pub use resolver::{CanvasWidgetTreeResolver, ResolvedCanvas};
pub use types::{
    CanvasRecord, CanvasView, Operation, RgbaColor, SceneItem, Transform2D, Value, ViewComponent,
};
