//! Default "switched on" state values for game-state-bound widgets.
//!
//! [`DefaultValueRegistry`] is the authoritative source of literal defaults for
//! runtime-bound UI widgets keyed by DataCore binding paths.

mod registry;
#[cfg(test)]
mod tests;

pub use registry::DefaultValueRegistry;
