use lopdf::Document;
use qpdf::QPdf;
use tiny_poppler::PdfPasswords;

pub(crate) fn unlock_pdf(doc: &Document, passwords: Option<&PdfPasswords>) -> Result<(), String> {
    // Check if encrypted and encryption state is still None (i.e., not yet authenticated)
    let is_encrypted = doc.is_encrypted() && doc.encryption_state.is_none();

    match (passwords, is_encrypted) {
        (Some(pwds), true) => {
            if let Some(user_pwd) = &pwds.user {
                doc.authenticate_password(user_pwd)
                    .map_err(|err| err.to_string())
            } else if let Some(owner_pwd) = &pwds.owner {
                doc.authenticate_owner_password(owner_pwd)
                    .map_err(|err| err.to_string())
            } else {
                Err("No password provided to unlock the encrypted PDF.".to_string())
            }
        }
        (None, true) => Err("PDF is encrypted but no passwords were provided.".to_string()),
        _ => Ok(()),
    }
}

pub(crate) fn ensure_pdf_output(output_path: &std::path::Path) -> Result<(), String> {
    // check ending with .pdf
    if output_path.extension().and_then(|s| s.to_str()) != Some("pdf") {
        return Err(format!(
            "Output file must have a .pdf extension: {}",
            output_path.display()
        ));
    }
    Ok(())
}

pub(crate) fn open_maybe_locked(
    buffers: &[u8],
    passwords: Option<&PdfPasswords>,
) -> Result<QPdf, String> {
    match passwords {
        Some(pwds) => {
            if let Some(user_pwd) = &pwds.user {
                QPdf::read_from_memory_encrypted(buffers, user_pwd)
                    .map_err(|e| format!("Failed to open PDF with user password: {}", e))
            } else if let Some(owner_pwd) = &pwds.owner {
                QPdf::read_from_memory_encrypted(buffers, owner_pwd)
                    .map_err(|e| format!("Failed to open PDF with owner password: {}", e))
            } else {
                // no password provided, try without
                QPdf::read_from_memory(buffers).map_err(|e| format!("Failed to open PDF: {}", e))
            }
        }
        None => {
            // Try processing without a password
            QPdf::read_from_memory(buffers).map_err(|e| format!("Failed to open PDF: {}", e))
        }
    }
}
