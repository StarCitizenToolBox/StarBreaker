//! SWF candidate derivation, selection, and import-merge loading.

mod candidates;
mod flash_paths;
mod loader;
#[cfg(test)]
mod tests;

pub use flash_paths::flash_swf_candidates;

pub(super) use candidates::build_swf_selection_manifest;
pub(super) use loader::load_first_swf;
