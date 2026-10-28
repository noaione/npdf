use clap::Args;
use color_eyre::{Result, eyre::Context};
use color_print::cprintln;
use lopdf::Document;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tiny_poppler::PdfPasswords;

use crate::common::{NpdfError, ensure_pdf_output, unlock_pdf};
use crate::pdf::prepress_black::patch_prepress_black;

#[derive(Args, Debug, PartialEq)]
pub struct FixBlackArgs {
    /// Path to the PDF file to fix.
    pub pdf: PathBuf,
    /// Output file path to save the fixed PDF.
    ///
    /// Required unless `--dry-run` is given.
    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,
    /// Report what would change without writing the output file.
    #[arg(long)]
    pub dry_run: bool,
    /// Also patch K-only separations whose alternate is DeviceCMYK.
    #[arg(long)]
    pub include_cmyk: bool,
}

pub fn run(args: FixBlackArgs, passwords: Option<&PdfPasswords>) -> Result<()> {
    if !args.pdf.exists() {
        return Err(NpdfError::MissingPdfFile(args.pdf.display().to_string()).into());
    }

    if !args.dry_run {
        let Some(output) = args.output.as_ref() else {
            return Err(NpdfError::RequireArgumentWhen("--output", "--dry-run").into());
        };
        ensure_pdf_output(output)?;
    }

    cprintln!("<magenta,bold>Loading PDF</>: {}", args.pdf.display());
    let mut doc = Document::load(&args.pdf)?;
    unlock_pdf(&doc, passwords)?;

    let report =
        patch_prepress_black(&mut doc, args.include_cmyk).context("patching prepress black")?;

    if args.dry_run {
        print_spaces(&report);
    }

    cprintln!("<magenta,bold>Result</>: {}", report.summary());
    for note in &report.notes {
        cprintln!("<yellow>note</>: {}", note);
    }

    if args.dry_run {
        return Ok(());
    }

    let output = args
        .output
        .as_ref()
        .expect("checked before loading the document");

    if !report.changed() {
        cprintln!(
            "<yellow>Nothing to patch; not writing</>: {}",
            output.display()
        );
        return Ok(());
    }

    cprintln!("<magenta,bold>Saving output PDF</>: {}", output.display());
    doc.save(output)?;

    Ok(())
}

/// One line per distinct colourspace, with how many copies of it were found.
fn print_spaces(report: &crate::pdf::prepress_black::Report) {
    if report.spaces.is_empty() {
        cprintln!("<cyan>No Separation or DeviceN colourspaces found.</>");
        return;
    }

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for space in &report.spaces {
        *counts.entry(space.describe()).or_insert(0) += 1;
    }
    for (line, count) in counts {
        if count == 1 {
            cprintln!("<cyan>{line}</>");
        } else {
            cprintln!("<cyan>{line}</> <dim>(x{count})</>");
        }
    }
}
