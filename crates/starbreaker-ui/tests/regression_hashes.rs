//! Pixel-stable regression tests for canvas-composed PNGs.
//!
//! **Phase 11 status: dormant.** The seeded SHA-256 hashes from Phase 10 only
//! proved reproducibility of blank output (see `docs/ui-plan2.md` Phase 10.5
//! reality check). New hashes will be seeded in Phase 15 once the layout +
//! paint engines (Phases 12–14) produce content that visually matches the
//! references in `reference/in-game/Clipper/`.
//!
//! Until then this file deliberately contains no `#[test]` functions so
//! `cargo test` cannot incidentally re-bless wrong output.
