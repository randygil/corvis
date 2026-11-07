//! Magic-number detection of file types, like the `file-type` package that
//! `FileTypeValidator` uses in the Node.js ecosystem (`fileTypeFromBuffer(buffer)?.mime`).
//!
//! Detected formats: JPEG, PNG/APNG, GIF, WebP, BMP, ICO/CUR, TIFF, AVIF/HEIC (ISO-BMFF `ftyp`),
//! PSD, JPEG XL, PDF, PostScript, RTF, ZIP (and DOCX/XLSX/PPTX/EPUB/ODF/JAR by their entries),
//! GZIP, BZIP2, 7z, RAR, XZ, ZSTD, TAR, MP3, MP4/M4A/MOV/3GP, WAV, AVI, OGG (Vorbis/Opus/Theora),
//! FLAC, WebM/Matroska, MIDI, WASM, SQLite, ELF, EXE, Mach-O, WOFF/WOFF2, TTF, OTF, XML and Parquet.
//! Like `file-type`, text formats without a signature (CSV, plain text, JSON, SVG) are not detected.

/// Returns the MIME type `file-type` would report, or `None` if the format is unknown.
pub fn detect_mime(b: &[u8]) -> Option<&'static str> {
    let starts = |sig: &[u8]| b.len() >= sig.len() && &b[..sig.len()] == sig;
    let at = |off: usize, sig: &[u8]| b.len() >= off + sig.len() && &b[off..off + sig.len()] == sig;

    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if starts(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        // APNG has an `acTL` chunk before the first `IDAT`.
        let mut off = 8;
        while off + 8 <= b.len() {
            let len = u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]) as usize;
            let kind = &b[off + 4..off + 8];
            if kind == b"acTL" {
                return Some("image/apng");
            }
            if kind == b"IDAT" || kind == b"IEND" {
                break;
            }
            off += 12 + len;
        }
        return Some("image/png");
    }
    if starts(b"GIF8") {
        return Some("image/gif");
    }
    if starts(b"RIFF") && at(8, b"WEBP") {
        return Some("image/webp");
    }
    if starts(b"RIFF") && at(8, b"WAVE") {
        return Some("audio/wav");
    }
    if starts(b"RIFF") && at(8, b"AVI") {
        return Some("video/vnd.avi");
    }
    if starts(b"%PDF") {
        return Some("application/pdf");
    }
    if starts(b"PK") && b.len() > 3 && matches!((b[2], b[3]), (3, 4) | (5, 6) | (7, 8)) {
        return Some(zip_kind(b));
    }
    if starts(&[0x1F, 0x8B, 0x08]) {
        return Some("application/gzip");
    }
    if starts(b"BZh") {
        return Some("application/x-bzip2");
    }
    if starts(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return Some("application/x-7z-compressed");
    }
    if starts(b"Rar!\x1a\x07") {
        return Some("application/x-rar-compressed");
    }
    if starts(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        return Some("application/x-xz");
    }
    if starts(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return Some("application/zstd");
    }
    if starts(b"BM") && b.len() > 14 && b[6..10] == [0, 0, 0, 0] {
        return Some("image/bmp");
    }
    if starts(&[0, 0, 1, 0]) {
        return Some("image/x-icon");
    }
    if starts(&[0, 0, 2, 0]) {
        return Some("image/x-icon");
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        return Some("image/tiff");
    }
    if starts(b"8BPS") {
        return Some("image/vnd.adobe.photoshop");
    }
    if starts(&[0xFF, 0x0A]) || starts(&[0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A]) {
        return Some("image/jxl");
    }
    if at(4, b"ftyp") && b.len() >= 12 {
        return Some(ftyp_kind(&b[8..12]));
    }
    if starts(b"OggS") {
        if at(28, b"OpusHead") {
            return Some("audio/ogg; codecs=opus");
        }
        if at(28, b"\x80theora") {
            return Some("video/ogg");
        }
        return Some("audio/ogg");
    }
    if starts(b"fLaC") {
        return Some("audio/x-flac");
    }
    if starts(&[0x1A, 0x45, 0xDF, 0xA3]) {
        let head = &b[..b.len().min(4096)];
        if find(head, b"webm").is_some() {
            return Some("video/webm");
        }
        return Some("video/matroska");
    }
    if starts(b"ID3") || (b.len() > 1 && b[0] == 0xFF && (b[1] & 0xE6) == 0xE2) {
        return Some("audio/mpeg");
    }
    if starts(b"MThd") {
        return Some("audio/midi");
    }
    if starts(b"\0asm") {
        return Some("application/wasm");
    }
    if starts(b"SQLite format 3\0") {
        return Some("application/x-sqlite3");
    }
    if starts(b"\x7fELF") {
        return Some("application/x-elf");
    }
    if starts(b"MZ") {
        return Some("application/x-msdownload");
    }
    if starts(&[0xCF, 0xFA, 0xED, 0xFE]) || starts(&[0xFE, 0xED, 0xFA, 0xCF]) || starts(&[0xCA, 0xFE, 0xBA, 0xBE]) {
        return Some("application/x-mach-binary");
    }
    if starts(b"wOFF") {
        return Some("font/woff");
    }
    if starts(b"wOF2") {
        return Some("font/woff2");
    }
    if starts(&[0x00, 0x01, 0x00, 0x00, 0x00]) {
        return Some("font/ttf");
    }
    if starts(b"OTTO") {
        return Some("font/otf");
    }
    if starts(b"{\\rtf") {
        return Some("application/rtf");
    }
    if starts(b"%!PS") {
        return Some("application/postscript");
    }
    if starts(b"PAR1") {
        return Some("application/vnd.apache.parquet");
    }
    if at(257, b"ustar") {
        return Some("application/x-tar");
    }
    if starts(b"<?xml ") || starts(b"\xEF\xBB\xBF<?xml ") {
        return Some("application/xml");
    }
    None
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    memchr::memmem::find(hay, needle)
}

fn ftyp_kind(brand: &[u8]) -> &'static str {
    let brand = String::from_utf8_lossy(brand).replace('\0', " ");
    let brand = brand.trim();
    match brand {
        "avif" | "avis" => "image/avif",
        "mif1" | "msf1" | "heic" | "heix" | "hevc" | "hevx" => "image/heic",
        "qt" => "video/quicktime",
        "M4V" | "M4VH" | "M4VP" => "video/x-m4v",
        "M4P" => "video/mp4",
        "M4B" => "audio/mp4",
        "M4A" => "audio/x-m4a",
        "F4V" => "video/mp4",
        "F4P" => "video/mp4",
        "F4A" => "audio/mp4",
        "F4B" => "audio/mp4",
        "crx" => "image/x-canon-cr3",
        b if b.starts_with("3g2") => "video/3gpp2",
        b if b.starts_with("3g") => "video/3gpp",
        b if b.starts_with("jp2") => "image/jp2",
        _ => "video/mp4",
    }
}

/// ZIP-based formats by their entries (`[Content_Types].xml` + `word/`, `xl/`, `ppt/`; `mimetype`).
fn zip_kind(b: &[u8]) -> &'static str {
    let mut off = 0usize;
    let mut has_content_types = false;
    while off + 30 <= b.len() && &b[off..off + 4] == b"PK\x03\x04" {
        let csize = u32::from_le_bytes([b[off + 18], b[off + 19], b[off + 20], b[off + 21]]) as usize;
        let nlen = u16::from_le_bytes([b[off + 26], b[off + 27]]) as usize;
        let xlen = u16::from_le_bytes([b[off + 28], b[off + 29]]) as usize;
        let name_end = (off + 30 + nlen).min(b.len());
        let name = &b[off + 30..name_end];
        let data_start = name_end + xlen;
        if name == b"mimetype" && data_start <= b.len() {
            let data = &b[data_start..(data_start + csize).min(b.len())];
            match data {
                b"application/epub+zip" => return "application/epub+zip",
                b"application/vnd.oasis.opendocument.text" => return "application/vnd.oasis.opendocument.text",
                b"application/vnd.oasis.opendocument.spreadsheet" => {
                    return "application/vnd.oasis.opendocument.spreadsheet";
                }
                b"application/vnd.oasis.opendocument.presentation" => {
                    return "application/vnd.oasis.opendocument.presentation";
                }
                _ => {}
            }
        }
        if name == b"[Content_Types].xml" {
            has_content_types = true;
        }
        if name.starts_with(b"word/") {
            return "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        }
        if name.starts_with(b"xl/") {
            return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
        }
        if name.starts_with(b"ppt/") {
            return "application/vnd.openxmlformats-officedocument.presentationml.presentation";
        }
        if name == b"META-INF/MANIFEST.MF" {
            return "application/java-archive";
        }
        // Entries with a data descriptor do not declare their size: stop there.
        let flags = u16::from_le_bytes([b[off + 6], b[off + 7]]);
        if flags & 0x08 != 0 && csize == 0 {
            break;
        }
        off = data_start + csize;
    }
    let _ = has_content_types;
    "application/zip"
}

#[cfg(test)]
mod tests {
    use super::detect_mime;

    #[test]
    fn detects_common_signatures() {
        assert_eq!(detect_mime(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Some("image/jpeg"));
        assert_eq!(detect_mime(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Some("image/png"));
        assert_eq!(detect_mime(b"%PDF-1.7\n"), Some("application/pdf"));
        assert_eq!(detect_mime(b"GIF89a"), Some("image/gif"));
        assert_eq!(detect_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(detect_mime(b"hello world"), None);
        assert_eq!(detect_mime(b""), None);
    }
}
