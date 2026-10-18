//! Photo hygiene (PRD §11 Privacy): location metadata is stripped on the
//! device before upload, and again here as a second line of defence.
//!
//! JPEG: drop APP1 (EXIF/XMP, where GPS lives) and APP13 (IPTC) segments.
//! PNG: drop `eXIf`, `tEXt`, `iTXt` and `zTXt` chunks. Anything else passes
//! through untouched; the upload route only accepts these two and WebP.

pub fn strip_metadata(bytes: &[u8]) -> Vec<u8> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        strip_jpeg(bytes)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        strip_png(bytes)
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        strip_webp(bytes)
    } else {
        bytes.to_vec()
    }
}

pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn strip_jpeg(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    out.extend_from_slice(&b[..2]);
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            // Not at a marker: malformed; keep the rest as is.
            out.extend_from_slice(&b[i..]);
            return out;
        }
        let marker = b[i + 1];
        // Start of scan: the image data follows; copy everything.
        if marker == 0xDA {
            out.extend_from_slice(&b[i..]);
            return out;
        }
        let len = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        let end = (i + 2 + len).min(b.len());
        if !matches!(marker, 0xE1 | 0xED) {
            out.extend_from_slice(&b[i..end]);
        }
        i = end;
    }
    out.extend_from_slice(&b[i.min(b.len())..]);
    out
}

fn strip_png(b: &[u8]) -> Vec<u8> {
    let mut out = b[..8].to_vec();
    let mut i = 8;
    while i + 12 <= b.len() {
        let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let kind = &b[i + 4..i + 8];
        let end = (i + 12 + len).min(b.len());
        if !matches!(kind, b"eXIf" | b"tEXt" | b"iTXt" | b"zTXt") {
            out.extend_from_slice(&b[i..end]);
        }
        i = end;
    }
    out
}

/// WebP: drop `EXIF` and `XMP ` chunks. The VP8X flags still advertise them,
/// which decoders tolerate; the data itself is gone.
fn strip_webp(b: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(b.len());
    let mut i = 12;
    while i + 8 <= b.len() {
        let len = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let end = (i + 8 + len + (len & 1)).min(b.len());
        if !matches!(&b[i..i + 4], b"EXIF" | b"XMP ") {
            body.extend_from_slice(&b[i..end]);
        }
        i = end;
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    out
}

/// A one-page text PDF, enough for a donation receipt (FR-49). Helvetica,
/// WinAnsi: the receipt is ASCII apart from the rupiah figures, which are.
pub fn text_pdf(title: &str, lines: &[String]) -> Vec<u8> {
    fn esc(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                '(' | ')' | '\\' => format!("\\{c}"),
                c if c.is_ascii() => c.to_string(),
                '–' | '—' => "-".into(),
                '·' => "-".into(),
                _ => "?".into(),
            })
            .collect()
    }
    let mut content = String::from("BT\n/F2 18 Tf\n56 780 Td\n");
    content.push_str(&format!("({}) Tj\n/F1 11 Tf\n0 -28 Td\n", esc(title)));
    for l in lines {
        content.push_str(&format!("({}) Tj\n0 -17 Td\n", esc(l)));
    }
    content.push_str("ET\n");

    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_jpeg_exif_and_keeps_the_image() {
        // SOI, APP0(len 4), APP1(len 6 "GPS!"), SOS, data, EOI
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46, 0xFF, 0xE1, 0x00, 0x06, b'G', b'P',
            b'S', b'!', 0xFF, 0xDA, 0x00, 0x02, 0x11, 0x22, 0xFF, 0xD9,
        ];
        let out = strip_metadata(&jpeg);
        assert!(!out.windows(4).any(|w| w == b"GPS!"));
        assert!(out.ends_with(&[0x11, 0x22, 0xFF, 0xD9]));
        assert!(out.starts_with(&[0xFF, 0xD8, 0xFF, 0xE0]));
    }

    #[test]
    fn drops_png_text_chunks() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let chunk = |kind: &[u8], data: &[u8]| {
            let mut c = (data.len() as u32).to_be_bytes().to_vec();
            c.extend_from_slice(kind);
            c.extend_from_slice(data);
            c.extend_from_slice(&[0, 0, 0, 0]);
            c
        };
        png.extend(chunk(b"IHDR", &[1; 13]));
        png.extend(chunk(b"tEXt", b"GPS=-6.2"));
        png.extend(chunk(b"IEND", &[]));
        let out = strip_metadata(&png);
        assert!(!out.windows(4).any(|w| w == b"tEXt"));
        assert!(out.windows(4).any(|w| w == b"IEND"));
    }

    #[test]
    fn writes_a_parseable_pdf() {
        let pdf = text_pdf("Receipt (DON-1)", &["Rp108,000 - 3 box".into()]);
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.contains("\\(DON-1\\)"));
        assert!(s.trim_end().ends_with("%%EOF"));
    }
}
