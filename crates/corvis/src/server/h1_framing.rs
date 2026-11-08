//! A framing check in front of hyper: a sans-IO state machine that follows where each HTTP/1 request
//! starts and ends on one connection, and refuses the framings that two parsers can read differently
//! (bare LF line endings, whitespace after a chunk size, `Content-Length` together with
//! `Transfer-Encoding`, bare LF in the trailer section).
//!
//! The check only decides where a message ends. Header syntax, token rules, limits and status codes
//! stay with hyper. Every message it lets through has exactly one reading, so hyper finds the same
//! boundaries; what it does not fully understand (a bad `Transfer-Encoding`, an invalid or differing
//! `Content-Length`, `HTTP/1.0` with `Transfer-Encoding`, an unknown version, an oversized head) is
//! handed to hyper unchanged and the stream ends right after that head, so no later request exists
//! for the two parsers to disagree about.
//!
//! This module must not be combined with hyper's `ignore_invalid_headers`: httparse would skip header
//! lines that the check reads (see `design.md`, decision 4, of the change
//! `e0-reject-ambiguous-http1-framing`).

use memchr::memchr;

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
/// hyper's `DecodedLength::MAX_LEN`.
const MAX_LEN: u64 = u64::MAX - 2;
/// Bytes held after an `Upgrade` request before the check stops reading the socket.
pub(super) const HOLD_CAP: usize = 64 * 1024;
/// Two bytes that hyper rejects in every position of the framing: at the start of a head httparse
/// answers a parse error (`400`), and every chunked decoder state that is not chunk data wants `\n`
/// after `\r`.
const REJECT_BYTES: &[u8] = b"\r\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ending {
    /// Send the rejection bytes, then end of stream.
    Reject,
    /// End of stream.
    End,
}

/// The end of the connection as far as hyper is concerned, delayed until `wait` requests have finished.
#[derive(Debug, Clone, Copy)]
pub(super) struct Terminal {
    pub ending: Ending,
    pub wait: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum St {
    /// First bytes of the connection: a prefix of the HTTP/2 preface is held in `head`.
    Preface,
    /// Between messages: empty lines are skipped.
    Start,
    /// A CR of an empty line was seen and is held until the LF.
    StartCr,
    /// Buffering a request head in `head`.
    Head,
    /// `Content-Length` body: bytes left.
    Body(u64),
    /// Digits of a chunk size.
    ChunkSize,
    /// Chunk extension, up to the CR.
    ChunkExt,
    /// The LF that ends the chunk size line.
    ChunkSizeLf,
    /// Chunk data: bytes left.
    ChunkData(u64),
    /// The CR after chunk data.
    ChunkCr,
    /// The LF after chunk data.
    ChunkLf,
    /// Start of a trailer line (or of the empty line that ends the trailer section).
    TrailerStart,
    /// Inside a trailer line.
    TrailerLine,
    /// The LF that ends a trailer line.
    TrailerLineLf,
    /// The LF of the empty line that ends the trailer section.
    TrailerEndLf,
    /// After an `Upgrade` request: bytes are kept in `held`, not parsed, until the request is answered.
    Hold,
    /// HTTP/2 or an upgraded connection: everything is forwarded.
    Passthrough,
    /// A terminal was set: later input is ignored.
    Done,
}

#[derive(Debug, PartialEq, Eq)]
enum BodyKind {
    None,
    Length(u64),
    Chunked,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Reject,
    Handoff,
    Request { body: BodyKind, hold: bool },
}

pub(super) struct Framing {
    st: St,
    max_head: usize,
    head: Vec<u8>,
    held: Vec<u8>,
    prev_cr: bool,
    line_len: usize,
    chunk: u64,
    chunk_digit: bool,
    hold_after: bool,
    /// Requests forwarded to hyper whose framing the check understood.
    heads: u64,
    /// Messages whose end the check reached (tests only).
    #[cfg(test)]
    completed: u64,
    terminal: Option<Terminal>,
    terminal_sent: bool,
}

impl Framing {
    /// `max_head` is the head size hyper accepts (its `max_buf_size`).
    pub(super) fn new(max_head: usize) -> Self {
        Framing {
            st: St::Preface,
            max_head,
            head: Vec::new(),
            held: Vec::new(),
            prev_cr: false,
            line_len: 0,
            chunk: 0,
            chunk_digit: false,
            hold_after: false,
            heads: 0,
            #[cfg(test)]
            completed: 0,
            terminal: None,
            terminal_sent: false,
        }
    }

    pub(super) fn is_holding(&self) -> bool {
        self.st == St::Hold
    }

    pub(super) fn hold_full(&self) -> bool {
        self.held.len() >= HOLD_CAP
    }

    /// Requests that must have finished before held bytes are looked at.
    pub(super) fn heads(&self) -> u64 {
        self.heads
    }

    /// The end of the connection, if it was reached and not yet sent.
    pub(super) fn pending_terminal(&self) -> Option<Terminal> {
        if self.terminal_sent { None } else { self.terminal }
    }

    pub(super) fn terminal_sent(&self) -> bool {
        self.terminal_sent
    }

    /// Writes the rejection bytes (if any) and marks the end of the connection as sent.
    pub(super) fn send_terminal(&mut self, out: &mut Vec<u8>) {
        if let Some(t) = self.terminal
            && t.ending == Ending::Reject
        {
            out.extend_from_slice(REJECT_BYTES);
        }
        self.terminal_sent = true;
    }

    fn terminate(&mut self, ending: Ending, mid_body: bool) {
        let wait = if mid_body { self.heads.saturating_sub(1) } else { self.heads };
        self.terminal = Some(Terminal { ending, wait });
        self.st = St::Done;
        self.head = Vec::new();
        self.held = Vec::new();
    }

    /// The held bytes go to hyper unchanged, and so does everything after them (the connection was
    /// upgraded).
    pub(super) fn release(&mut self, out: &mut Vec<u8>) {
        if self.st == St::Hold {
            out.append(&mut self.held);
            self.held = Vec::new();
            self.st = St::Passthrough;
        }
    }

    /// The upgrade request was answered without switching protocols: the held bytes are the start of
    /// the next request.
    pub(super) fn resume(&mut self, out: &mut Vec<u8>) {
        if self.st == St::Hold {
            let held = std::mem::take(&mut self.held);
            self.st = St::Start;
            self.feed(&held, out);
        }
    }

    /// End of input: whatever is buffered goes to hyper as it is.
    pub(super) fn eof(&mut self, out: &mut Vec<u8>) {
        match self.st {
            St::Preface | St::Head => {
                out.append(&mut self.head);
                self.st = St::Start;
            }
            St::StartCr => {
                out.push(b'\r');
                self.st = St::Start;
            }
            _ => {}
        }
    }

    /// Length of the prefix of `data` that can be forwarded as it is (a body, chunk data or a
    /// pass-through connection), advancing the state past it.
    pub(super) fn passthrough_prefix(&mut self, data: &[u8]) -> usize {
        match self.st {
            St::Passthrough => data.len(),
            St::Body(n) => {
                let k = n.min(data.len() as u64);
                if k == n {
                    self.finish_message();
                } else {
                    self.st = St::Body(n - k);
                }
                k as usize
            }
            St::ChunkData(n) => {
                let k = n.min(data.len() as u64);
                self.st = if k == n { St::ChunkCr } else { St::ChunkData(n - k) };
                k as usize
            }
            _ => 0,
        }
    }

    fn finish_message(&mut self) {
        #[cfg(test)]
        {
            self.completed += 1;
        }
        self.st = if std::mem::take(&mut self.hold_after) { St::Hold } else { St::Start };
    }

    /// Runs `input` through the machine, appending to `out` the bytes that go to hyper, in order.
    pub(super) fn feed(&mut self, mut input: &[u8], out: &mut Vec<u8>) {
        while !input.is_empty() {
            match self.st {
                St::Done => return,
                St::Passthrough => {
                    out.extend_from_slice(input);
                    return;
                }
                St::Hold => {
                    self.held.extend_from_slice(input);
                    return;
                }
                St::Preface => {
                    let matched = self.head.len();
                    let n = input.len().min(PREFACE.len() - matched);
                    if input[..n] == PREFACE[matched..matched + n] {
                        self.head.extend_from_slice(&input[..n]);
                        input = &input[n..];
                        if self.head.len() == PREFACE.len() {
                            out.append(&mut self.head);
                            self.st = St::Passthrough;
                        }
                    } else {
                        let held = std::mem::take(&mut self.head);
                        self.st = St::Start;
                        self.feed(&held, out);
                    }
                }
                St::Start => match input[0] {
                    b'\n' => {
                        out.push(b'\n');
                        input = &input[1..];
                    }
                    b'\r' => {
                        self.st = St::StartCr;
                        input = &input[1..];
                    }
                    _ => {
                        self.st = St::Head;
                        self.prev_cr = false;
                        self.line_len = 0;
                        self.head.clear();
                    }
                },
                St::StartCr => {
                    if input[0] == b'\n' {
                        out.extend_from_slice(b"\r\n");
                        input = &input[1..];
                        self.st = St::Start;
                    } else {
                        self.terminate(Ending::Reject, false);
                    }
                }
                St::Head => input = self.scan_head(input, out),
                St::Body(n) => {
                    let k = n.min(input.len() as u64);
                    out.extend_from_slice(&input[..k as usize]);
                    input = &input[k as usize..];
                    if k == n {
                        self.finish_message();
                    } else {
                        self.st = St::Body(n - k);
                    }
                }
                St::ChunkData(n) => {
                    let k = n.min(input.len() as u64);
                    out.extend_from_slice(&input[..k as usize]);
                    input = &input[k as usize..];
                    self.st = if k == n { St::ChunkCr } else { St::ChunkData(n - k) };
                }
                _ => {
                    let b = input[0];
                    input = &input[1..];
                    self.framing_byte(b, out);
                }
            }
        }
    }

    /// One byte of chunked framing (chunk size line, chunk end, trailer section).
    fn framing_byte(&mut self, b: u8, out: &mut Vec<u8>) {
        match self.st {
            St::ChunkSize => match b {
                b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' => {
                    let digit = (b as char).to_digit(16).map_or(0, u64::from);
                    match self.chunk.checked_mul(16).and_then(|v| v.checked_add(digit)) {
                        Some(v) => {
                            self.chunk = v;
                            self.chunk_digit = true;
                            out.push(b);
                        }
                        None => self.terminate(Ending::Reject, true),
                    }
                }
                b';' if self.chunk_digit => {
                    out.push(b);
                    self.st = St::ChunkExt;
                }
                b'\r' if self.chunk_digit => {
                    out.push(b);
                    self.st = St::ChunkSizeLf;
                }
                // Whitespace, a missing size, any other byte.
                _ => self.terminate(Ending::Reject, true),
            },
            St::ChunkExt => match b {
                b'\r' => {
                    out.push(b);
                    self.st = St::ChunkSizeLf;
                }
                b'\n' => self.terminate(Ending::Reject, true),
                _ => out.push(b),
            },
            St::ChunkSizeLf => {
                if b == b'\n' {
                    out.push(b);
                    self.st = if self.chunk == 0 { St::TrailerStart } else { St::ChunkData(self.chunk) };
                } else {
                    self.terminate(Ending::Reject, true);
                }
            }
            St::ChunkCr => {
                if b == b'\r' {
                    out.push(b);
                    self.st = St::ChunkLf;
                } else {
                    self.terminate(Ending::Reject, true);
                }
            }
            St::ChunkLf => {
                if b == b'\n' {
                    out.push(b);
                    self.start_chunk();
                } else {
                    self.terminate(Ending::Reject, true);
                }
            }
            St::TrailerStart => match b {
                b'\r' => {
                    out.push(b);
                    self.st = St::TrailerEndLf;
                }
                b'\n' => self.terminate(Ending::Reject, true),
                _ => {
                    out.push(b);
                    self.st = St::TrailerLine;
                }
            },
            St::TrailerLine => match b {
                b'\r' => {
                    out.push(b);
                    self.st = St::TrailerLineLf;
                }
                b'\n' => self.terminate(Ending::Reject, true),
                _ => out.push(b),
            },
            St::TrailerLineLf => {
                if b == b'\n' {
                    out.push(b);
                    self.st = St::TrailerStart;
                } else {
                    self.terminate(Ending::Reject, true);
                }
            }
            St::TrailerEndLf => {
                if b == b'\n' {
                    out.push(b);
                    self.finish_message();
                } else {
                    self.terminate(Ending::Reject, true);
                }
            }
            _ => unreachable!("framing_byte is only called in chunked framing states"),
        }
    }

    fn start_chunk(&mut self) {
        self.st = St::ChunkSize;
        self.chunk = 0;
        self.chunk_digit = false;
    }

    /// Buffers a request head until its empty line. Returns the input after the head.
    fn scan_head<'a>(&mut self, data: &'a [u8], out: &mut Vec<u8>) -> &'a [u8] {
        let mut i = 0;
        let mut complete = false;
        let mut bad = false;
        while i < data.len() {
            let b = data[i];
            i += 1;
            match b {
                b'\r' => {
                    if self.prev_cr {
                        bad = true;
                        break;
                    }
                    self.prev_cr = true;
                }
                b'\n' => {
                    if !self.prev_cr {
                        bad = true;
                        break;
                    }
                    self.prev_cr = false;
                    if self.line_len == 0 {
                        complete = true;
                        break;
                    }
                    self.line_len = 0;
                }
                _ => {
                    if self.prev_cr {
                        bad = true;
                        break;
                    }
                    self.line_len += 1;
                }
            }
        }
        if bad {
            self.terminate(Ending::Reject, false);
            return &[];
        }
        self.head.extend_from_slice(&data[..i]);
        if complete {
            self.head_complete(out);
        } else if self.head.len() >= self.max_head {
            out.append(&mut self.head);
            self.terminate(Ending::End, false);
            return &[];
        }
        &data[i..]
    }

    fn head_complete(&mut self, out: &mut Vec<u8>) {
        match analyze(&self.head) {
            Verdict::Reject => self.terminate(Ending::Reject, false),
            Verdict::Handoff => {
                out.append(&mut self.head);
                self.terminate(Ending::End, false);
            }
            Verdict::Request { body, hold } => {
                out.extend_from_slice(&self.head);
                self.head.clear();
                if self.head.capacity() > 8 * 1024 {
                    self.head = Vec::new();
                }
                self.heads += 1;
                self.hold_after = hold;
                match body {
                    BodyKind::None | BodyKind::Length(0) => self.finish_message(),
                    BodyKind::Length(n) => self.st = St::Body(n),
                    BodyKind::Chunked => self.start_chunk(),
                }
            }
        }
    }
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = s {
        s = rest;
    }
    while let [rest @ .., b' ' | b'\t'] = s {
        s = rest;
    }
    s
}

/// `Content-Length` value as hyper reads it: comma separated, each `1*DIGIT`, all equal.
fn content_length(value: &[u8]) -> Option<u64> {
    let mut found: Option<u64> = None;
    for part in value.split(|&b| b == b',') {
        let part = trim(part);
        if part.is_empty() {
            return None;
        }
        let mut n: u64 = 0;
        for &d in part {
            if !d.is_ascii_digit() {
                return None;
            }
            n = n.checked_mul(10)?.checked_add(u64::from(d - b'0'))?;
        }
        if n > MAX_LEN {
            return None;
        }
        match found {
            Some(prev) if prev != n => return None,
            _ => found = Some(n),
        }
    }
    found
}

/// `true` when the last coding of a `Transfer-Encoding` value is `chunked` (hyper's `is_chunked_`).
fn ends_in_chunked(value: &[u8]) -> bool {
    let last = value.rsplit(|&b| b == b',').next().unwrap_or(&[]);
    trim(last).eq_ignore_ascii_case(b"chunked")
}

/// Reads the length headers of a head that is known to have only CRLF line ends.
fn analyze(head: &[u8]) -> Verdict {
    let mut pos = 0;
    let mut line_no = 0;
    let mut http11 = false;
    let mut hold = false;
    let mut cl_seen = false;
    let mut cl_bad = false;
    let mut cl: Option<u64> = None;
    let mut te_seen = false;
    let mut te_chunked = false;
    while pos < head.len() {
        let Some(lf) = memchr(b'\n', &head[pos..]) else { break };
        // The scan guarantees CRLF: the byte before every LF is a CR.
        let line = &head[pos..pos + lf - 1];
        pos += lf + 1;
        if line.is_empty() {
            break;
        }
        if line_no == 0 {
            line_no = 1;
            http11 = line.ends_with(b" HTTP/1.1");
            if !http11 && !line.ends_with(b" HTTP/1.0") {
                return Verdict::Handoff;
            }
            let method = line.split(|&b| b == b' ').next().unwrap_or(&[]);
            hold = method == b"CONNECT";
            continue;
        }
        let Some(colon) = memchr(b':', line) else { continue };
        let name = &line[..colon];
        let value = trim(&line[colon + 1..]);
        if name.eq_ignore_ascii_case(b"content-length") {
            cl_seen = true;
            match content_length(value) {
                Some(n) if cl.is_none_or(|prev| prev == n) => cl = Some(n),
                _ => cl_bad = true,
            }
        } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
            te_seen = true;
            te_chunked = ends_in_chunked(value);
        } else if name.eq_ignore_ascii_case(b"upgrade") {
            hold = true;
        }
    }
    if cl_seen && te_seen {
        return Verdict::Reject;
    }
    let body = if te_seen {
        if !http11 || !te_chunked {
            return Verdict::Handoff;
        }
        BodyKind::Chunked
    } else if cl_seen {
        match cl {
            Some(n) if !cl_bad => BodyKind::Length(n),
            _ => return Verdict::Handoff,
        }
    } else {
        BodyKind::None
    };
    Verdict::Request { body, hold }
}

#[cfg(test)]
mod tests;
