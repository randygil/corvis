//! `multipart/form-data` parser with the exact semantics of `busboy` 1.6 (the parser used by
//! `multer`): boundary search, part headers (including RFC 2231/5987 `filename*`), field and file
//! size limits, `files`/`fields`/`parts` limits and the same error messages.
//!
//! The request body is already buffered by the server, so the parser runs over the whole body
//! and produces the list of events busboy would emit, in the same order.

use bytes::{Bytes, BytesMut};

use super::Limits;

/// An event emitted while parsing, in busboy order.
#[derive(Debug)]
pub(crate) enum Event {
    /// A file part started (`busboy.on('file')`).
    FileStart { id: usize, name: Option<String>, filename: Option<String>, encoding: String, mime_type: String },
    /// The file reached `limits.fileSize` (`fileStream.on('limit')`).
    FileLimit { id: usize },
    /// The file part ended; `data` is its (possibly truncated) content.
    FileEnd { id: usize, data: Bytes },
    /// A non-file field (`busboy.on('field')`).
    Field { name: Option<String>, value: String, value_truncated: bool },
    FilesLimit,
    FieldsLimit,
    PartsLimit,
    /// `busboy.on('error')` (message without the `Multipart: ` prefix that the HTTP mapping adds).
    Error(&'static str),
}

pub(crate) const BOUNDARY_NOT_FOUND: &str = "Multipart: Boundary not found";
pub(crate) const MALFORMED_PART_HEADER: &str = "Malformed part header";
pub(crate) const UNEXPECTED_END_OF_FORM: &str = "Unexpected end of form";
pub(crate) const UNEXPECTED_END_OF_FILE: &str = "Unexpected end of file";

/// Configuration error raised before parsing (busboy throws in its constructor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfigError {
    MissingContentType,
    MalformedContentType,
    Unsupported(String),
    BoundaryNotFound,
}

impl ConfigError {
    pub(crate) fn message(&self) -> String {
        match self {
            ConfigError::MissingContentType => "Missing Content-Type".into(),
            ConfigError::MalformedContentType => "Malformed content type".into(),
            ConfigError::Unsupported(ct) => format!("Unsupported content type: {ct}"),
            ConfigError::BoundaryNotFound => BOUNDARY_NOT_FOUND.into(),
        }
    }
}

pub(crate) struct Config<'a> {
    pub limits: &'a Limits,
    pub preserve_path: bool,
    /// `defParamCharset` (multer passes `latin1` by default).
    pub def_param_charset: &'a str,
}

/// Parses `Content-Type` like busboy (`parseContentType`). Parameter values are latin1 strings.
pub(crate) fn parse_content_type(s: &[u8]) -> Option<(String, String, Vec<(String, String)>)> {
    if s.is_empty() {
        return None;
    }
    let mut i = 0;
    while i < s.len() {
        if !is_token(s[i]) {
            if s[i] != b'/' || i == 0 {
                return None;
            }
            break;
        }
        i += 1;
    }
    if i == s.len() {
        return None;
    }
    let ty = latin1(&s[..i]).to_ascii_lowercase();
    i += 1;
    let sub_start = i;
    let mut params = Vec::new();
    while i < s.len() {
        if !is_token(s[i]) {
            if i == sub_start {
                return None;
            }
            params = parse_params(s, i, false, &|v, _| Some(latin1(&v)))?;
            break;
        }
        i += 1;
    }
    if i == sub_start {
        return None;
    }
    let sub = latin1(&s[sub_start..i]).to_ascii_lowercase();
    Some((ty, sub, params))
}

/// Validates the request `Content-Type` and returns the boundary.
pub(crate) fn boundary_of(content_type: Option<&[u8]>) -> Result<Vec<u8>, ConfigError> {
    let Some(ct) = content_type else { return Err(ConfigError::MissingContentType) };
    let Some((ty, sub, params)) = parse_content_type(ct) else { return Err(ConfigError::MalformedContentType) };
    if ty != "multipart" || sub != "form-data" {
        return Err(ConfigError::Unsupported(latin1(ct)));
    }
    params
        .into_iter()
        .find(|(k, _)| k == "boundary")
        .map(|(_, v)| v.chars().map(|c| c as u32 as u8).collect())
        .ok_or(ConfigError::BoundaryNotFound)
}

/// Parses the multipart body.
pub(crate) fn parse(body: &Bytes, boundary: &[u8], cfg: &Config<'_>) -> Vec<Event> {
    // busboy writes a CRLF first ("just in case there is no preamble").
    let mut hay = BytesMut::with_capacity(body.len() + 2);
    hay.extend_from_slice(b"\r\n");
    hay.extend_from_slice(body);
    let hay = hay.freeze();
    let mut needle = Vec::with_capacity(boundary.len() + 4);
    needle.extend_from_slice(b"\r\n--");
    needle.extend_from_slice(boundary);

    let mut p = Parser::new(cfg);
    let finder = memchr::memmem::Finder::new(&needle);
    let mut pos = 0usize;
    while pos <= hay.len() && !p.ignore {
        match finder.find(&hay[pos..]) {
            Some(off) => {
                let m = pos + off;
                p.on_chunk(&hay, pos, m, true);
                pos = m + needle.len();
            }
            None => {
                if pos < hay.len() {
                    p.on_chunk(&hay, pos, hay.len(), false);
                }
                break;
            }
        }
    }
    // `_final`: an incomplete form is an error; otherwise check the end state.
    if !p.complete {
        p.events.push(Event::Error(UNEXPECTED_END_OF_FORM));
    } else if p.hparser_active {
        p.events.push(Event::Error(MALFORMED_PART_HEADER));
    } else if p.file.is_some() {
        p.events.push(Event::Error(UNEXPECTED_END_OF_FILE));
    }
    p.events
}

struct CurrentFile {
    id: usize,
    size: u64,
    chunks: Vec<Bytes>,
}

struct Parser<'a> {
    cfg: &'a Config<'a>,
    events: Vec<Event>,
    hparser: HeaderParser,
    hparser_active: bool,
    match_post_boundary: u8,
    complete: bool,
    ignore: bool,
    skip_part: bool,
    parts: i64,
    fields: u64,
    files: u64,
    hit_files_limit: bool,
    hit_fields_limit: bool,
    next_file_id: usize,
    file: Option<CurrentFile>,
    field: Option<Vec<u8>>,
    field_size: u64,
    part_charset: String,
    part_name: Option<String>,
    part_truncated: bool,
}

impl<'a> Parser<'a> {
    fn new(cfg: &'a Config<'a>) -> Self {
        Parser {
            cfg,
            events: Vec::new(),
            hparser: HeaderParser::new(),
            hparser_active: false,
            match_post_boundary: 0,
            complete: false,
            ignore: false,
            skip_part: false,
            parts: -1,
            fields: 0,
            files: 0,
            hit_files_limit: false,
            hit_fields_limit: false,
            next_file_id: 0,
            file: None,
            field: None,
            field_size: 0,
            part_charset: "utf8".into(),
            part_name: None,
            part_truncated: false,
        }
    }

    fn limit(v: Option<u64>, default: u64) -> u64 {
        v.unwrap_or(default)
    }

    fn parts_limit(&self) -> i64 {
        self.cfg.limits.parts.map(|p| p.min(i64::MAX as u64) as i64).unwrap_or(i64::MAX)
    }

    /// `ssCb(isMatch, data, start, end)`.
    fn on_chunk(&mut self, data: &Bytes, mut start: usize, end: usize, is_match: bool) {
        if self.ignore {
            return;
        }
        loop {
            if self.hparser_active {
                match self.hparser.push(&data[..], start, end) {
                    HeaderPush::Error => {
                        self.hparser_active = false;
                        self.hparser.reset();
                        self.events.push(Event::Error(MALFORMED_PART_HEADER));
                        break;
                    }
                    HeaderPush::Partial(p) => start = p,
                    HeaderPush::Done(p, header) => {
                        self.hparser_active = false;
                        self.on_header(header);
                        start = p;
                    }
                }
            }
            if start >= end {
                break;
            }
            if self.match_post_boundary != 0 {
                if self.match_post_boundary == 1 {
                    match data[start] {
                        b'-' => {
                            self.match_post_boundary = 2;
                            start += 1;
                        }
                        b'\r' => {
                            self.match_post_boundary = 3;
                            start += 1;
                        }
                        _ => self.match_post_boundary = 0,
                    }
                    if start == end {
                        return;
                    }
                }
                if self.match_post_boundary == 2 {
                    self.match_post_boundary = 0;
                    if data[start] == b'-' {
                        // End of multipart data.
                        self.complete = true;
                        self.ignore = true;
                        return;
                    }
                    self.on_chunk(&Bytes::from_static(b"-"), 0, 1, false);
                } else if self.match_post_boundary == 3 {
                    self.match_post_boundary = 0;
                    if data[start] == b'\n' {
                        start += 1;
                        if self.parts >= self.parts_limit() {
                            break;
                        }
                        self.hparser_active = true;
                        if start == end {
                            break;
                        }
                        continue;
                    }
                    self.on_chunk(&Bytes::from_static(b"\r"), 0, 1, false);
                }
            }
            if !self.skip_part {
                if let Some(file) = self.file.as_mut() {
                    let limit = Self::limit(self.cfg.limits.file_size, u64::MAX);
                    let actual = ((end - start) as u64).min(limit - file.size) as usize;
                    let chunk = data.slice(start..start + actual);
                    file.size += actual as u64;
                    if file.size == limit {
                        if !chunk.is_empty() {
                            file.chunks.push(chunk);
                        }
                        let id = file.id;
                        self.events.push(Event::FileLimit { id });
                        self.skip_part = true;
                    } else {
                        file.chunks.push(chunk);
                    }
                } else if let Some(field) = self.field.as_mut() {
                    let limit = Self::limit(self.cfg.limits.field_size, 1024 * 1024);
                    let actual = ((end - start) as u64).min(limit - self.field_size) as usize;
                    field.extend_from_slice(&data[start..start + actual]);
                    self.field_size += actual as u64;
                    if self.field_size == limit {
                        self.skip_part = true;
                        self.part_truncated = true;
                    }
                }
            }
            break;
        }

        if is_match {
            self.match_post_boundary = 1;
            if let Some(file) = self.file.take() {
                let data = match file.chunks.len() {
                    0 => Bytes::new(),
                    1 => file.chunks.into_iter().next().unwrap(),
                    _ => {
                        let mut b = BytesMut::with_capacity(file.size as usize);
                        for c in &file.chunks {
                            b.extend_from_slice(c);
                        }
                        b.freeze()
                    }
                };
                self.events.push(Event::FileEnd { id: file.id, data });
            } else if let Some(field) = self.field.take() {
                let value = decode_text(&field, &self.part_charset);
                self.field_size = 0;
                self.events.push(Event::Field {
                    name: self.part_name.clone(),
                    value,
                    value_truncated: self.part_truncated,
                });
            }
            self.parts += 1;
            if self.parts == self.parts_limit() {
                self.events.push(Event::PartsLimit);
            }
        }
    }

    fn on_header(&mut self, header: Vec<(String, Vec<Vec<u8>>)>) {
        self.skip_part = false;
        let mut part_type = "text/plain".to_string();
        self.part_charset = "utf8".into();
        let mut part_encoding = "7bit".to_string();
        self.part_name = None;
        self.part_truncated = false;
        let get = |n: &str| header.iter().find(|(k, _)| k == n).map(|(_, v)| &v[0]);

        let Some(disp) = get("content-disposition") else {
            self.skip_part = true;
            return;
        };
        let charset = self.cfg.def_param_charset;
        let Some((dtype, params)) = parse_disposition(disp, charset) else {
            self.skip_part = true;
            return;
        };
        if dtype != "form-data" {
            self.skip_part = true;
            return;
        }
        let mut filename: Option<String> = None;
        let param = |n: &str| params.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
        if let Some(name) = param("name").filter(|n| !n.is_empty()) {
            self.part_name = Some(name);
        }
        if let Some(f) = param("filename*").filter(|f| !f.is_empty()) {
            filename = Some(f);
        } else if let Some(f) = param("filename").filter(|f| !f.is_empty()) {
            filename = Some(f);
        }
        if let Some(f) = filename.as_mut() {
            if !self.cfg.preserve_path {
                *f = basename(f);
            }
        }
        if let Some(ct) = get("content-type") {
            if let Some((t, s, p)) = parse_content_type(ct) {
                part_type = format!("{t}/{s}");
                if let Some((_, cs)) = p.iter().find(|(k, _)| k == "charset") {
                    self.part_charset = cs.to_lowercase();
                }
            }
        }
        if let Some(enc) = get("content-transfer-encoding") {
            part_encoding = latin1(enc).to_lowercase();
        }

        if part_type == "application/octet-stream" || filename.is_some() {
            if self.files == Self::limit(self.cfg.limits.files, u64::MAX) {
                if !self.hit_files_limit {
                    self.hit_files_limit = true;
                    self.events.push(Event::FilesLimit);
                }
                self.skip_part = true;
                return;
            }
            self.files += 1;
            let id = self.next_file_id;
            self.next_file_id += 1;
            self.file = Some(CurrentFile { id, size: 0, chunks: Vec::new() });
            self.events.push(Event::FileStart {
                id,
                name: self.part_name.clone(),
                filename,
                encoding: part_encoding,
                mime_type: part_type,
            });
        } else {
            if self.fields == Self::limit(self.cfg.limits.fields, u64::MAX) {
                if !self.hit_fields_limit {
                    self.hit_fields_limit = true;
                    self.events.push(Event::FieldsLimit);
                }
                self.skip_part = true;
                return;
            }
            self.fields += 1;
            self.field = Some(Vec::new());
            self.field_size = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// Part header parser (`HeaderParser` in busboy).
// ---------------------------------------------------------------------------

const MAX_HEADER_PAIRS: usize = 2000;
const MAX_HEADER_SIZE: usize = 16 * 1024;

enum HeaderPush {
    Error,
    Partial(usize),
    Done(usize, Vec<(String, Vec<Vec<u8>>)>),
}

struct HeaderParser {
    header: Vec<(String, Vec<Vec<u8>>)>,
    pair_count: usize,
    byte_count: usize,
    state: u8,
    name: Vec<u8>,
    value: Vec<u8>,
    crlf: u8,
}

impl HeaderParser {
    fn new() -> Self {
        HeaderParser { header: Vec::new(), pair_count: 0, byte_count: 0, state: 0, name: Vec::new(), value: Vec::new(), crlf: 0 }
    }

    fn reset(&mut self) {
        *self = HeaderParser::new();
    }

    fn push(&mut self, chunk: &[u8], mut pos: usize, end: usize) -> HeaderPush {
        let mut start = pos;
        while pos < end {
            // HPARSER_NAME
            if self.state == 0 {
                let mut done = false;
                while pos < end {
                    if self.byte_count == MAX_HEADER_SIZE {
                        return HeaderPush::Error;
                    }
                    self.byte_count += 1;
                    let code = chunk[pos];
                    if !is_token(code) {
                        if code != b':' {
                            return HeaderPush::Error;
                        }
                        self.name.extend_from_slice(&chunk[start..pos]);
                        if self.name.is_empty() {
                            return HeaderPush::Error;
                        }
                        pos += 1;
                        done = true;
                        self.state = 1;
                        break;
                    }
                    pos += 1;
                }
                if !done {
                    self.name.extend_from_slice(&chunk[start..pos]);
                    break;
                }
            }
            // HPARSER_PRE_OWS
            if self.state == 1 {
                let mut done = false;
                while pos < end {
                    if self.byte_count == MAX_HEADER_SIZE {
                        return HeaderPush::Error;
                    }
                    self.byte_count += 1;
                    let code = chunk[pos];
                    if code != b' ' && code != b'\t' {
                        start = pos;
                        done = true;
                        self.state = 2;
                        break;
                    }
                    pos += 1;
                }
                if !done {
                    break;
                }
            }
            // HPARSER_VALUE
            match self.crlf {
                0 => {
                    while pos < end {
                        if self.byte_count == MAX_HEADER_SIZE {
                            return HeaderPush::Error;
                        }
                        self.byte_count += 1;
                        let code = chunk[pos];
                        if !is_field_vchar(code) {
                            if code != b'\r' {
                                return HeaderPush::Error;
                            }
                            self.crlf += 1;
                            break;
                        }
                        pos += 1;
                    }
                    self.value.extend_from_slice(&chunk[start..pos.min(end)]);
                    pos += 1;
                }
                1 => {
                    if self.byte_count == MAX_HEADER_SIZE {
                        return HeaderPush::Error;
                    }
                    self.byte_count += 1;
                    if chunk[pos] != b'\n' {
                        return HeaderPush::Error;
                    }
                    pos += 1;
                    self.crlf += 1;
                }
                2 => {
                    if self.byte_count == MAX_HEADER_SIZE {
                        return HeaderPush::Error;
                    }
                    self.byte_count += 1;
                    let code = chunk[pos];
                    if code == b' ' || code == b'\t' {
                        // Folded value.
                        start = pos;
                        self.crlf = 0;
                    } else {
                        self.pair_count += 1;
                        if self.pair_count < MAX_HEADER_PAIRS {
                            let name = latin1(&self.name).to_lowercase();
                            let value = std::mem::take(&mut self.value);
                            match self.header.iter_mut().find(|(k, _)| *k == name) {
                                Some((_, v)) => v.push(value),
                                None => self.header.push((name, vec![value])),
                            }
                        }
                        if code == b'\r' {
                            self.crlf += 1;
                            pos += 1;
                        } else {
                            start = pos;
                            self.crlf = 0;
                            self.state = 0;
                            self.name.clear();
                            self.value.clear();
                        }
                    }
                }
                _ => {
                    if self.byte_count == MAX_HEADER_SIZE {
                        return HeaderPush::Error;
                    }
                    self.byte_count += 1;
                    if chunk[pos] != b'\n' {
                        return HeaderPush::Error;
                    }
                    pos += 1;
                    let header = std::mem::take(&mut self.header);
                    self.reset();
                    return HeaderPush::Done(pos, header);
                }
            }
        }
        HeaderPush::Partial(pos.min(end))
    }
}

// ---------------------------------------------------------------------------
// Content-Disposition / parameter parsing (busboy `utils.js`).
// ---------------------------------------------------------------------------

/// `parseDisposition(str, defDecoder)`. `s` holds the raw header bytes (busboy reads headers as latin1).
fn parse_disposition(s: &[u8], def_charset: &str) -> Option<(String, Vec<(String, String)>)> {
    if s.is_empty() {
        return None;
    }
    let mut i = 0;
    let mut params = Vec::new();
    while i < s.len() {
        if !is_token(s[i]) {
            let def = |v: Vec<u8>, hint: u8| decode_param(&v, def_charset, hint);
            params = parse_params(s, i, true, &def)?;
            break;
        }
        i += 1;
    }
    let ty_end = s.iter().position(|c| !is_token(*c)).unwrap_or(s.len());
    Some((latin1(&s[..ty_end]).to_ascii_lowercase(), params))
}

/// Shared implementation of `parseContentTypeParams` / `parseDispositionParams`.
/// `decode(value_bytes, hint)` converts a non-extended value.
fn parse_params(
    s: &[u8],
    mut i: usize,
    extended: bool,
    decode: &dyn Fn(Vec<u8>, u8) -> Option<String>,
) -> Option<Vec<(String, String)>> {
    let mut params: Vec<(String, String)> = Vec::new();
    while i < s.len() {
        while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
            i += 1;
        }
        if i == s.len() {
            break;
        }
        if s[i] != b';' {
            return None;
        }
        i += 1;
        while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
            i += 1;
        }
        if i == s.len() {
            return None;
        }
        let name_start = i;
        while i < s.len() {
            if !is_token(s[i]) {
                if s[i] == b'=' {
                    break;
                }
                return None;
            }
            i += 1;
        }
        if i == s.len() {
            return None;
        }
        let name = &s[name_start..i];
        let value: String;
        if extended && name.last() == Some(&b'*') {
            // Extended value: charset'lang'percent-encoded.
            i += 1;
            let cs_start = i;
            while i < s.len() {
                if !is_charset(s[i]) {
                    if s[i] != b'\'' {
                        return None;
                    }
                    break;
                }
                i += 1;
            }
            if i == s.len() {
                return None;
            }
            let charset = latin1(&s[cs_start..i]);
            i += 1;
            while i < s.len() && s[i] != b'\'' {
                i += 1;
            }
            if i == s.len() {
                return None;
            }
            i += 1;
            if i == s.len() {
                return None;
            }
            let mut bytes = Vec::new();
            let mut encode = 0u8;
            while i < s.len() {
                let c = s[i];
                if !is_extended_value(c) {
                    if c == b'%' {
                        if i + 2 < s.len() {
                            if let (Some(h), Some(l)) = (hex(s[i + 1]), hex(s[i + 2])) {
                                let b = (h << 4) + l;
                                bytes.push(b);
                                i += 3;
                                if b >= 128 {
                                    encode = 2;
                                } else if encode == 0 {
                                    encode = 1;
                                }
                                continue;
                            }
                        }
                        return None;
                    }
                    break;
                }
                bytes.push(c);
                i += 1;
            }
            value = decode_param(&bytes, &charset, encode)?;
        } else {
            i += 1;
            if i == s.len() {
                return None;
            }
            let mut raw = Vec::new();
            if s[i] == b'"' {
                i += 1;
                let mut escaping = false;
                while i < s.len() {
                    let c = s[i];
                    if c == b'\\' {
                        if escaping {
                            raw.push(c);
                            escaping = false;
                        } else {
                            escaping = true;
                        }
                        i += 1;
                        continue;
                    }
                    if c == b'"' {
                        if escaping {
                            raw.push(c);
                            escaping = false;
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    if escaping {
                        // busboy keeps the backslash before a non-special char.
                        raw.push(b'\\');
                        escaping = false;
                    }
                    if !is_qdtext(c) {
                        return None;
                    }
                    raw.push(c);
                    i += 1;
                }
                if i == s.len() {
                    return None;
                }
                i += 1;
            } else {
                let vs = i;
                while i < s.len() {
                    if !is_token(s[i]) {
                        if i == vs {
                            return None;
                        }
                        break;
                    }
                    i += 1;
                }
                raw.extend_from_slice(&s[vs..i]);
            }
            value = decode(raw, 2)?;
        }
        let name = latin1(name).to_lowercase();
        if !params.iter().any(|(k, _)| *k == name) {
            params.push((name, value));
        }
    }
    Some(params)
}

/// `getDecoder(charset)(data, hint)` for parameter values (`data` = latin1 bytes).
fn decode_param(bytes: &[u8], charset: &str, hint: u8) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    match normalize_charset(charset).as_str() {
        "utf8" => {
            if hint < 2 {
                Some(latin1(bytes))
            } else {
                Some(String::from_utf8_lossy(bytes).into_owned())
            }
        }
        "latin1" => Some(latin1(bytes)),
        "utf16le" => Some(utf16le(bytes)),
        "base64" => {
            use base64::Engine as _;
            Some(base64::engine::general_purpose::STANDARD.encode(bytes))
        }
        _ => Some(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// `convertToUTF8(buffer, charset)` for field values.
fn decode_text(bytes: &[u8], charset: &str) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    match normalize_charset(charset).as_str() {
        "latin1" => latin1(bytes),
        "utf16le" => utf16le(bytes),
        "base64" => {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(bytes)
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn normalize_charset(c: &str) -> String {
    match c.to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" => "utf8".into(),
        "latin1" | "ascii" | "us-ascii" | "iso-8859-1" | "iso8859-1" | "iso88591" | "iso_8859-1" | "windows-1252"
        | "iso_8859-1:1987" | "cp1252" | "x-cp1252" => "latin1".into(),
        "utf16le" | "utf-16le" | "ucs2" | "ucs-2" => "utf16le".into(),
        "base64" => "base64".into(),
        other => other.to_string(),
    }
}

fn utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// Bytes read as latin1 (each byte is one code point), like `buffer.latin1Slice()`.
pub(crate) fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// busboy's `basename`: strips everything up to the last `/` or `\`; `.` and `..` become empty.
pub(crate) fn basename(path: &str) -> String {
    let tail = match path.rfind(['/', '\\']) {
        Some(i) => &path[i + 1..],
        None => path,
    };
    if tail == ".." || tail == "." { String::new() } else { tail.to_string() }
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn is_token(c: u8) -> bool {
    matches!(c, b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'^'..=b'z' | b'|' | b'~')
}

fn is_field_vchar(c: u8) -> bool {
    c == b'\t' || (c >= 0x20 && c != 0x7f)
}

fn is_qdtext(c: u8) -> bool {
    c == b'\t' || (c >= 0x20 && c != b'"' && c != b'\\' && c != 0x7f)
}

fn is_charset(c: u8) -> bool {
    matches!(c, b'!' | b'#'..=b'&' | b'+' | b'-' | b'0'..=b'9' | b'A'..=b'Z' | b'^'..=b'z' | b'{' | b'}' | b'~')
}

fn is_extended_value(c: u8) -> bool {
    matches!(c, b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'^'..=b'z' | b'|' | b'~')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_parse(body: &str, limits: Limits) -> Vec<Event> {
        let cfg = Config { limits: &limits, preserve_path: false, def_param_charset: "latin1" };
        parse(&Bytes::from(body.to_string()), b"XX", &cfg)
    }

    #[test]
    fn token_tables_match_busboy() {
        // Spot checks against the TOKEN / CHARSET / EXTENDED_VALUE tables of busboy.
        for c in [b'"', b'(', b')', b',', b'/', b':', b';', b'<', b'=', b'>', b'?', b'@', b'[', b'\\', b']', b'{', b'}', b' '] {
            assert!(!is_token(c), "{}", c as char);
        }
        for c in [b'!', b'#', b'$', b'%', b'&', b'\'', b'*', b'+', b'-', b'.', b'^', b'_', b'`', b'|', b'~', b'a', b'Z', b'0'] {
            assert!(is_token(c), "{}", c as char);
        }
        assert!(!is_charset(b'\'') && is_charset(b'%') && is_charset(b'{') && !is_charset(b'|') && !is_charset(b'*'));
        assert!(!is_extended_value(b'%') && !is_extended_value(b'\'') && !is_extended_value(b'*') && is_extended_value(b'|'));
    }

    #[test]
    fn parses_fields_and_files() {
        let body = "--XX\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nhola\r\n--XX\r\nContent-Disposition: form-data; name=\"f\"; filename=\"C:\\\\dir\\\\x.txt\"\r\nContent-Type: text/plain\r\n\r\nfile body\r\n--XX--\r\n";
        let ev = cfg_parse(body, Limits::default());
        assert!(matches!(&ev[0], Event::Field { name: Some(n), value, .. } if n == "a" && value == "hola"));
        assert!(matches!(&ev[1], Event::FileStart { filename: Some(f), mime_type, .. } if f == "x.txt" && mime_type == "text/plain"));
        assert!(matches!(&ev[2], Event::FileEnd { data, .. } if &data[..] == b"file body"));
        assert_eq!(ev.len(), 3);
    }

    #[test]
    fn unexpected_end_and_malformed_header() {
        let ev = cfg_parse("--XX\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nhola", Limits::default());
        assert!(matches!(ev.last(), Some(Event::Error(m)) if *m == UNEXPECTED_END_OF_FORM));
        let ev = cfg_parse("--XX\r\nBad Header\r\n\r\nx\r\n--XX--", Limits::default());
        assert!(matches!(ev.first(), Some(Event::Error(m)) if *m == MALFORMED_PART_HEADER));
    }

    #[test]
    fn file_size_limit_is_inclusive_like_busboy() {
        let body = "--XX\r\nContent-Disposition: form-data; name=\"f\"; filename=\"a.bin\"\r\n\r\n12345\r\n--XX--";
        let ev = cfg_parse(body, Limits { file_size: Some(5), ..Default::default() });
        assert!(ev.iter().any(|e| matches!(e, Event::FileLimit { .. })));
        let ev = cfg_parse(body, Limits { file_size: Some(6), ..Default::default() });
        assert!(!ev.iter().any(|e| matches!(e, Event::FileLimit { .. })));
    }

    #[test]
    fn extended_filename_and_latin1_default() {
        let body = "--XX\r\nContent-Disposition: form-data; name=\"f\"; filename*=UTF-8''%C3%B1and%C3%BA.png\r\n\r\nx\r\n--XX--";
        let ev = cfg_parse(body, Limits::default());
        assert!(matches!(&ev[0], Event::FileStart { filename: Some(f), .. } if f == "ñandú.png"));
        let mut raw = b"--XX\r\nContent-Disposition: form-data; name=\"f\"; filename=\"".to_vec();
        raw.extend_from_slice("ñ.png".as_bytes());
        raw.extend_from_slice(b"\"\r\n\r\nx\r\n--XX--");
        let limits = Limits::default();
        let cfg = Config { limits: &limits, preserve_path: false, def_param_charset: "latin1" };
        let ev = parse(&Bytes::from(raw), b"XX", &cfg);
        assert!(matches!(&ev[0], Event::FileStart { filename: Some(f), .. } if f == "Ã±.png"));
    }

    #[test]
    fn boundary_detection() {
        assert_eq!(boundary_of(Some(b"multipart/form-data; boundary=abc")).unwrap(), b"abc");
        assert_eq!(boundary_of(Some(b"multipart/form-data; boundary=\"a b\"")).unwrap(), b"a b");
        assert_eq!(boundary_of(Some(b"multipart/form-data")), Err(ConfigError::BoundaryNotFound));
        assert!(matches!(boundary_of(Some(b"multipart/mixed; boundary=x")), Err(ConfigError::Unsupported(_))));
    }
}
