//! Manifest schema for generic UI regression targets.

mod runner;
#[cfg(test)]
mod tests;
mod types;

pub use runner::compare_manifest_targets_with_loader;
pub use types::{
    UI_REGRESSION_MANIFEST_SCHEMA_VERSION, UiRegressionCategory, UiRegressionManifest,
    UiRegressionRoi, UiRegressionRunError, UiRegressionTarget, UiRegressionTargetResult,
    UiRegressionTier,
};
