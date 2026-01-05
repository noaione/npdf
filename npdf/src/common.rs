use color_eyre::Result;
use lopdf::Document;
use tiny_poppler::PdfPasswords;

#[derive(thiserror::Error, Debug)]
pub enum NpdfError {
    #[error("PDF is encrypted but no passwords were provided.")]
    EncryptedPdfNoPassword,
    #[error("No password provided to unlock the encrypted PDF.")]
    NoPasswordProvided,
    #[error("Output file must have a .pdf extension: {0}")]
    InvalidPdfOutput(String),
    #[error("PDF file does not exist: {0}")]
    MissingPdfFile(String),
    #[error("DPI must be a positive integer, got: {0}")]
    InvalidDpi(f64),
    #[error("{0} is required when not using {1}.")]
    RequireArgumentWhen(&'static str, &'static str),
    #[error("{0} must be between {1} and {2}.")]
    MustBetween(&'static str, usize, usize),
    #[error("Failed to create output directory: {0}")]
    CreateOutputDirError(#[from] std::io::Error),
    #[error("PDF output path not set.")]
    OutputNotSet,
}

pub(crate) fn unlock_pdf(doc: &Document, passwords: Option<&PdfPasswords>) -> Result<()> {
    // Check if encrypted and encryption state is still None (i.e., not yet authenticated)
    let is_encrypted = doc.is_encrypted() && doc.encryption_state.is_none();

    match (passwords, is_encrypted) {
        (Some(pwds), true) => {
            if let Some(user_pwd) = &pwds.user {
                doc.authenticate_password(user_pwd)?;
                Ok(())
            } else if let Some(owner_pwd) = &pwds.owner {
                doc.authenticate_owner_password(owner_pwd)?;
                Ok(())
            } else {
                Err(NpdfError::NoPasswordProvided.into())
            }
        }
        (None, true) => Err(NpdfError::EncryptedPdfNoPassword.into()),
        _ => Ok(()),
    }
}

pub(crate) fn ensure_pdf_output(output_path: &std::path::Path) -> Result<()> {
    // check ending with .pdf
    if output_path.extension().and_then(|s| s.to_str()) != Some("pdf") {
        Err(NpdfError::InvalidPdfOutput(output_path.display().to_string()).into())
    } else {
        Ok(())
    }
}
