//! The compressed response body: wraps the original body and encodes it as it is polled.
//!
//! Memory and CPU are bounded per response: at most [`INPUT_SLICE`] bytes go through the encoder in
//! one poll (the task yields to the runtime between slices), the output of a poll is whatever the
//! encoder produced for that slice, and the body is never collected. When the source has nothing
//! ready the encoder is flushed, so a slow stream is not held back by the encoder's buffers.

use std::io::{self, Write};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use flate2::Compression;
use flate2::write::{GzEncoder, ZlibEncoder};
use hyper::body::{Body, Frame, SizeHint};

use crate::server::ResBody;

/// The most input one poll feeds to the encoder.
pub(super) const INPUT_SLICE: usize = 64 * 1024;

/// A content coding the compression can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Coding {
    Brotli,
    Gzip,
    /// The `deflate` coding of HTTP: the zlib format (RFC 1950), like Node's `createDeflate`.
    Deflate,
}

impl Coding {
    pub(super) fn name(self) -> &'static str {
        match self {
            Coding::Brotli => "br",
            Coding::Gzip => "gzip",
            Coding::Deflate => "deflate",
        }
    }

    pub(super) fn from_name(name: &str) -> Option<Coding> {
        match name {
            "br" => Some(Coding::Brotli),
            "gzip" => Some(Coding::Gzip),
            "deflate" => Some(Coding::Deflate),
            _ => None,
        }
    }
}

/// The brotli window (log2 of its size) for a body: the smallest from 2^16 to 2^22 (Node's
/// default) that covers `len` bytes, or 2^20 when the length is unknown. A smaller window gives the
/// same decoded output with less memory per response.
pub(super) fn brotli_window(len: Option<u64>) -> u32 {
    match len {
        None => 20,
        Some(len) => (16..=22).find(|lgwin| (1u64 << lgwin) - 16 >= len).unwrap_or(22),
    }
}

enum Encoder {
    Gzip(GzEncoder<Vec<u8>>),
    Zlib(ZlibEncoder<Vec<u8>>),
    Brotli(Box<brotli::CompressorWriter<Vec<u8>>>),
}

impl Encoder {
    fn new(coding: Coding, level: u32, quality: u32, body_len: Option<u64>) -> Encoder {
        match coding {
            Coding::Gzip => Encoder::Gzip(GzEncoder::new(Vec::new(), Compression::new(level))),
            Coding::Deflate => Encoder::Zlib(ZlibEncoder::new(Vec::new(), Compression::new(level))),
            Coding::Brotli => Encoder::Brotli(Box::new(brotli::CompressorWriter::new(Vec::new(), 4096, quality, brotli_window(body_len)))),
        }
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            Encoder::Gzip(e) => e.write_all(data),
            Encoder::Zlib(e) => e.write_all(data),
            Encoder::Brotli(e) => e.write_all(data),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Encoder::Gzip(e) => e.flush(),
            Encoder::Zlib(e) => e.flush(),
            Encoder::Brotli(e) => e.flush(),
        }
    }

    /// The bytes produced so far.
    fn take(&mut self) -> Bytes {
        let out = match self {
            Encoder::Gzip(e) => std::mem::take(e.get_mut()),
            Encoder::Zlib(e) => std::mem::take(e.get_mut()),
            Encoder::Brotli(e) => std::mem::take(e.get_mut()),
        };
        Bytes::from(out)
    }

    /// Ends the stream and returns the last bytes.
    fn finish(self) -> io::Result<Bytes> {
        let out = match self {
            Encoder::Gzip(e) => e.finish()?,
            Encoder::Zlib(e) => e.finish()?,
            Encoder::Brotli(e) => e.into_inner(),
        };
        Ok(Bytes::from(out))
    }
}

/// A body whose data frames are the encoded form of another body.
pub(super) struct CompressedBody {
    inner: ResBody,
    encoder: Option<Encoder>,
    /// Input of the current data frame not yet fed to the encoder.
    pending_input: Bytes,
    /// Input was fed since the last flush.
    dirty: bool,
    source_done: bool,
    ended: bool,
}

impl CompressedBody {
    /// `body_len` is the length of the uncompressed body when it is known (it only sizes the
    /// brotli window).
    pub(super) fn new(inner: ResBody, coding: Coding, level: u32, brotli_quality: u32, body_len: Option<u64>) -> Self {
        CompressedBody {
            inner,
            encoder: Some(Encoder::new(coding, level, brotli_quality, body_len)),
            pending_input: Bytes::new(),
            dirty: false,
            source_done: false,
            ended: false,
        }
    }

    fn fail(&mut self, e: io::Error) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        self.encoder = None;
        self.ended = true;
        Poll::Ready(Some(Err(e)))
    }
}

fn data(bytes: Bytes) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
    Poll::Ready(Some(Ok(Frame::data(bytes))))
}

impl Body for CompressedBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = &mut *self;
        if this.ended {
            return Poll::Ready(None);
        }
        loop {
            let Some(encoder) = this.encoder.as_mut() else {
                this.ended = true;
                return Poll::Ready(None);
            };
            // Input left from the last data frame: one slice per poll.
            if !this.pending_input.is_empty() {
                let n = this.pending_input.len().min(INPUT_SLICE);
                let slice = this.pending_input.split_to(n);
                if let Err(e) = encoder.write(&slice) {
                    return this.fail(e);
                }
                this.dirty = true;
                let out = encoder.take();
                if !out.is_empty() {
                    return data(out);
                }
                // Nothing came out for this slice (the encoder is buffering): give the runtime a
                // turn before the next slice.
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            if this.source_done {
                let Some(encoder) = this.encoder.take() else { continue };
                return match encoder.finish() {
                    Ok(out) if !out.is_empty() => data(out),
                    Ok(_) => {
                        this.ended = true;
                        Poll::Ready(None)
                    }
                    Err(e) => this.fail(e),
                };
            }
            match Pin::new(&mut this.inner).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    // Trailers do not exist in Corvis bodies; they would not survive encoding anyway.
                    if let Ok(chunk) = frame.into_data() {
                        this.pending_input = chunk;
                    }
                }
                Poll::Ready(Some(Err(e))) => return this.fail(e),
                Poll::Ready(None) => this.source_done = true,
                Poll::Pending => {
                    if this.dirty {
                        this.dirty = false;
                        if let Err(e) = encoder.flush() {
                            return this.fail(e);
                        }
                        let out = encoder.take();
                        if !out.is_empty() {
                            return data(out);
                        }
                    }
                    return Poll::Pending;
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.ended
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::{BodyExt, StreamBody};

    #[tokio::test]
    async fn one_poll_feeds_at_most_one_slice() {
        let total = 1024 * 1024;
        let data = Bytes::from(vec![7u8; total]);
        let stream = futures::stream::iter(vec![Ok::<_, io::Error>(Frame::data(data))]);
        let mut body = CompressedBody::new(StreamBody::new(stream).boxed_unsync(), Coding::Gzip, 6, 4, Some(total as u64));
        // Poll until the first slice went through the encoder, and no further.
        std::future::poll_fn(|cx| match Pin::new(&mut body).poll_frame(cx) {
            Poll::Ready(Some(Ok(_))) => Poll::Ready(()),
            Poll::Pending if body.pending_input.len() < total => Poll::Ready(()),
            Poll::Pending => Poll::Pending,
            other => panic!("unexpected {other:?}"),
        })
        .await;
        assert_eq!(body.pending_input.len(), total - INPUT_SLICE);
    }
}
