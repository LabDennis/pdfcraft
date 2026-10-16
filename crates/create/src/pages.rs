//! Recover the PDF preview from a zipped Pages document; this does not render Pages/IWA.
use std::io::{Cursor, Read};

use crate::CreateError;

pub const PAGES_PREVIEW_NOTICE: &str =
    "Imported the embedded Pages PDF preview. It may be outdated or incomplete; export from Pages for a current, complete PDF.";
const MAX_PACKAGE: usize = 128 << 20;
const MAX_PREVIEW: u64 = 256 << 20;
const PREVIEW: &str = "QuickLook/Preview.pdf";

/// Read only the conventional PDF preview, never thumbnails, linked files or archive paths.
/// The caller must still validate the returned PDF and its copying permissions.
pub fn pages_pdf_preview(bytes: &[u8]) -> Result<Vec<u8>, CreateError> {
    read(bytes, MAX_PREVIEW)
}

fn read(bytes: &[u8], max_preview: u64) -> Result<Vec<u8>, CreateError> {
    let bad = |message: String| CreateError::Invalid(format!("Pages: {message}"));
    if bytes.len() > MAX_PACKAGE {
        return Err(bad("package exceeds 128 MiB".into()));
    }
    check_directory(bytes).map_err(|e| bad(e.into()))?;
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| bad(format!("not a readable ZIP package ({e})")))?;
    if zip.len() > 10_000 {
        return Err(bad("package contains too many entries".into()));
    }
    let file = zip.by_name(PREVIEW).map_err(|_| bad("no readable embedded PDF preview. Open this document in Pages and export it as PDF; IWA documents and image-only previews cannot be converted here.".into()))?;
    if file.size() == 0 || file.size() > max_preview {
        return Err(bad("PDF preview is empty or exceeds the size limit".into()));
    }
    let mut pdf = Vec::new();
    file.take(max_preview.saturating_add(1)).read_to_end(&mut pdf).map_err(|e| bad(format!("cannot read PDF preview ({e})")))?;
    if pdf.len() as u64 > max_preview {
        return Err(bad("PDF preview exceeds the size limit".into()));
    }
    let head = pdf.get(..pdf.len().min(1024)).unwrap_or_default();
    if !head.windows(5).any(|w| w == b"%PDF-") {
        return Err(bad("embedded preview is not a PDF".into()));
    }
    Ok(pdf)
}

// Check the ordinary ZIP footer before the library allocates its entry index. Pages previews
// do not need multi-disk or ZIP64 archives under our size limits (APPNOTE 4.3.16).
fn check_directory(bytes: &[u8]) -> Result<(), &'static str> {
    let tail = bytes.get(bytes.len().saturating_sub(65_557)..).unwrap_or_default();
    let u16_at = |b: &[u8], at: usize| b.get(at..at + 2).and_then(|s| <[u8; 2]>::try_from(s).ok()).map(u16::from_le_bytes);
    let (at, footer) = tail
        .windows(22)
        .enumerate()
        .rev()
        .find(|(at, h)| h.starts_with(b"PK\x05\x06") && u16_at(h, 20).is_some_and(|n| usize::from(n) == tail.len() - at - 22))
        .ok_or("not a readable ZIP package (missing footer)")?;
    let entries = u16_at(footer, 10).ok_or("invalid ZIP entry count")?;
    if entries > 10_000 {
        return Err("package contains too many entries");
    }
    if u16_at(footer, 4) != Some(0) || u16_at(footer, 6) != Some(0) || u16_at(footer, 8) != Some(entries) {
        return Err("multi-disk ZIP packages are unsupported");
    }
    if at.checked_sub(20).and_then(|i| tail.get(i..i + 4)) == Some(b"PK\x06\x07")
        || footer.get(12..16) == Some(&[255; 4])
        || footer.get(16..20) == Some(&[255; 4])
    {
        return Err("ZIP64 packages are unsupported; export from Pages as PDF");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn package(path: &str, data: &[u8]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(path, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        zip.write_all(data).unwrap();
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn pages_preview_requires_exact_pdf_entry_and_bounds_inflation() {
        let pdf = b"%PDF-1.7\noriginal preview bytes";
        assert_eq!(pages_pdf_preview(&package(PREVIEW, pdf)).unwrap(), pdf);
        for path in ["preview.jpg", "../QuickLook/Preview.pdf", "Data/embedded.pdf", "Index/Document.iwa"] {
            assert!(pages_pdf_preview(&package(path, pdf)).unwrap_err().to_string().contains("export it as PDF"));
        }
        assert!(pages_pdf_preview(&package(PREVIEW, b"not PDF")).is_err());
        assert!(read(&package(PREVIEW, pdf), 8).is_err());
        assert!(pages_pdf_preview(b"not a zip").is_err());
        let good = package(PREVIEW, pdf);
        let mut too_many = good.clone();
        let count = too_many.len() - 22 + 10;
        too_many[count..count + 2].copy_from_slice(&10_001u16.to_le_bytes());
        assert!(pages_pdf_preview(&too_many).unwrap_err().to_string().contains("too many entries"));
        for cut in (0..good.len()).step_by(7) {
            assert!(pages_pdf_preview(&good[..cut]).is_err());
        }
    }
}
