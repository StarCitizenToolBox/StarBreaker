//! Animation binding diagnostics CLI.
//!
//! Thin command-line front end over
//! [`starbreaker_3d::animation::binding_report`]: it reads local animation JSON
//! sidecars and source rig files, then delegates report construction to the
//! shared library. Report-only — it never mutates sidecars or guesses bindings.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde_json::Value;
use starbreaker_3d::animation::binding_report as report;

use crate::error::{CliError, Result};

#[derive(Subcommand)]
pub enum AnimationCommand {
    /// Build a report comparing animation JSON track hashes against rig, GLB,
    /// meshsetup, and chrparams names.
    BindingReport {
        /// Folder containing exported animation JSON files (scanned recursively).
        #[arg(long)]
        animation_folder: Option<PathBuf>,
        /// One or more individual animation JSON files to include.
        #[arg(long)]
        animation_json: Vec<PathBuf>,
        /// Source .cga file to inspect for NMC/rig node names.
        #[arg(long)]
        cga: Option<PathBuf>,
        /// Source .cgam file to inspect for NMC/rig node names.
        #[arg(long)]
        cgam: Option<PathBuf>,
        /// Source .meshsetup file to inspect for Joint names.
        #[arg(long)]
        meshsetup: Option<PathBuf>,
        /// Source .chrparams file to inspect for animation references.
        #[arg(long)]
        chrparams: Option<PathBuf>,
        /// Converted .glb file to inspect for exported node names.
        #[arg(long)]
        glb: Option<PathBuf>,
        /// Write machine-readable JSON report to this path (stdout if omitted).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Optional Markdown summary report path.
        #[arg(long)]
        markdown: Option<PathBuf>,
    },
}

impl AnimationCommand {
    pub fn run(self) -> Result<()> {
        match self {
            Self::BindingReport {
                animation_folder,
                animation_json,
                cga,
                cgam,
                meshsetup,
                chrparams,
                glb,
                out,
                markdown,
            } => binding_report(BindingReportArgs {
                animation_folder,
                animation_json,
                cga,
                cgam,
                meshsetup,
                chrparams,
                glb,
                out,
                markdown,
            }),
        }
    }
}

struct BindingReportArgs {
    animation_folder: Option<PathBuf>,
    animation_json: Vec<PathBuf>,
    cga: Option<PathBuf>,
    cgam: Option<PathBuf>,
    meshsetup: Option<PathBuf>,
    chrparams: Option<PathBuf>,
    glb: Option<PathBuf>,
    out: Option<PathBuf>,
    markdown: Option<PathBuf>,
}

fn binding_report(args: BindingReportArgs) -> Result<()> {
    let mut names = report::NameSources::default();
    let mut source_reports = Vec::<Value>::new();

    for (path, label) in [(args.cga.as_deref(), "cga"), (args.cgam.as_deref(), "cgam")] {
        if let Some(path) = path {
            let data = read_file(path)?;
            source_reports.push(report::rig_source_report(
                &data,
                label,
                &path.display().to_string(),
                &mut names,
            ));
        }
    }
    if let Some(path) = args.meshsetup.as_deref() {
        let text = read_text_lossy(path)?;
        source_reports.push(report::meshsetup_source_report(
            &text,
            &path.display().to_string(),
            &mut names,
        ));
    }
    if let Some(path) = args.chrparams.as_deref() {
        let text = read_text_lossy(path)?;
        source_reports.push(report::chrparams_source_report(
            &text,
            &path.display().to_string(),
            &mut names,
        ));
    }
    if let Some(path) = args.glb.as_deref() {
        let data = read_file(path)?;
        let report = report::glb_source_report(&data, &path.display().to_string(), &mut names)
            .ok_or_else(|| {
                CliError::InvalidInput(format!("failed to parse GLB JSON chunk: {}", path.display()))
            })?;
        source_reports.push(report);
    }

    let animation_files =
        collect_animation_json_files(args.animation_folder.as_deref(), &args.animation_json)?;
    if animation_files.is_empty() {
        return Err(CliError::InvalidInput(
            "no animation JSON files were supplied/found".into(),
        ));
    }

    let mut clip_stats = Vec::new();
    for file in &animation_files {
        let data = read_file(file)?;
        let value: Value = serde_json::from_slice(&data)?;
        clip_stats.extend(report::clips_from_value(&file.display().to_string(), &value));
    }

    let report_value = report::build_report(&names, source_reports, animation_files.len(), &clip_stats);
    let pretty = serde_json::to_string_pretty(&report_value)?;

    if let Some(out) = args.out.as_deref() {
        write_text(out, &pretty)?;
        eprintln!("Animation binding report written to {}", out.display());
    } else {
        println!("{pretty}");
    }

    if let Some(md_path) = args.markdown.as_deref() {
        write_text(md_path, &report::build_markdown_report(&report_value))?;
        eprintln!("Markdown summary written to {}", md_path.display());
    }

    Ok(())
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| CliError::IoPath {
                source: e,
                path: parent.display().to_string(),
            })?;
        }
    }
    std::fs::write(path, text).map_err(|e| CliError::IoPath {
        source: e,
        path: path.display().to_string(),
    })
}

fn collect_animation_json_files(folder: Option<&Path>, explicit: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in explicit {
        if path.is_file() {
            files.push(path.clone());
        } else {
            return Err(CliError::NotFound(format!(
                "animation JSON file not found: {}",
                path.display()
            )));
        }
    }
    if let Some(folder) = folder {
        if !folder.is_dir() {
            return Err(CliError::NotFound(format!(
                "animation folder not found: {}",
                folder.display()
            )));
        }
        collect_json_recursive(folder, &mut files)?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_json_recursive(folder: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(folder).map_err(|e| CliError::IoPath {
        source: e,
        path: folder.display().to_string(),
    })? {
        let entry = entry.map_err(|e| CliError::IoPath {
            source: e,
            path: folder.display().to_string(),
        })?;
        let path = entry.path();
        let ty = entry.file_type().map_err(|e| CliError::IoPath {
            source: e,
            path: path.display().to_string(),
        })?;
        if ty.is_dir() {
            collect_json_recursive(&path, files)?;
        } else if ty.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| CliError::IoPath {
        source: e,
        path: path.display().to_string(),
    })
}

fn read_text_lossy(path: &Path) -> Result<String> {
    let data = read_file(path)?;
    Ok(String::from_utf8_lossy(&data).into_owned())
}
