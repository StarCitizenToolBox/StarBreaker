//! UI render replay commands (UI Plan 2 stub).
//!
//! The MFD/SWF subcommands from the prior AVM1-execution experiment have been
//! removed. They will be replaced by canvas-driven static composition built
//! by the new `starbreaker-ui` crate (UI Plan 2, phases 2–9).
//!
//! See `/docs/ui-plan2.md`.

use clap::Subcommand;

use crate::error::Result;

#[derive(Subcommand, Debug)]
pub enum UiCommand {
    /// Render UI PNGs from an existing decomposed scene.json UI binding list.
    ///
    /// Disabled in UI Plan 2 Phase 0. Re-enabled in Phase 9 when the new
    /// composer is wired into the export pipeline.
    Render,
    /// Render a single MFD SWF file.
    ///
    /// Disabled in UI Plan 2. MFD rendering is being replaced by canvas
    /// composition; standalone SWF rendering is no longer a supported path.
    Mfd,
}

impl UiCommand {
    pub fn run(self) -> Result<()> {
        match self {
            UiCommand::Render => Err(crate::error::CliError::MissingRequirement(
                "starbreaker ui render: temporarily disabled during UI Plan 2 phase 0; \
                 will return in phase 9 when the canvas composer is wired in. \
                 See /docs/ui-plan2.md".to_string(),
            )),
            UiCommand::Mfd => Err(crate::error::CliError::MissingRequirement(
                "starbreaker ui mfd: removed in UI Plan 2. MFD content is now \
                 produced by canvas composition, not standalone SWF rendering. \
                 See /docs/ui-plan2.md".to_string(),
            )),
        }
    }
}
