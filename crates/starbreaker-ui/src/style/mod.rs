//! Manufacturer style loader: tint, CRT parameters, and palette hints.

pub(crate) mod colour_roles;
mod loader;
mod parse;
#[cfg(test)]
mod tests;
mod types;

pub use loader::StyleLoader;
pub use types::{CrtParams, ManufacturerStyle};
