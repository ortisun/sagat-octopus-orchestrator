//! Inspect each decrypted PostgreSQL frame header before the driver can allocate its body.
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, ready},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub(crate) struct BoundedStream<S> {
    inner: S,
    header: [u8; 5],
    header_read: usize,
    header_sent: usize,
    remaining: usize,
    max_frame: usize,
    budget: usize,
    exceeded: Arc<AtomicBool>,
}
impl<S> BoundedStream<S> {
    pub(crate) fn new(
        inner: S,
        max_frame: usize,
        budget: usize,
        exceeded: Arc<AtomicBool>,
    ) -> Self {
        Self {
            inner,
            header: [0; 5],
            header_read: 0,
            header_sent: 0,
            remaining: 0,
            max_frame,
            budget,
            exceeded,
        }
    }
    fn reject(&self) -> io::Error {
        self.exceeded.store(true, Ordering::Relaxed);
        io::Error::other("database response limit")
    }
}
impl<S: AsyncRead + Unpin> AsyncRead for BoundedStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.exceeded.load(Ordering::Relaxed) {
            return Poll::Ready(Err(this.reject()));
        }
        if out.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        while this.header_read < 5 {
            let mut buf = ReadBuf::new(&mut this.header[this.header_read..]);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut buf))?;
            let count = buf.filled().len();
            if count == 0 {
                return Poll::Ready(if this.header_read == 0 {
                    Ok(())
                } else {
                    Err(io::ErrorKind::UnexpectedEof.into())
                });
            }
            this.header_read += count;
            if this.header_read == 5 {
                let length = u32::from_be_bytes([
                    this.header[1],
                    this.header[2],
                    this.header[3],
                    this.header[4],
                ]) as usize;
                if length < 4 || length > this.max_frame || length + 1 > this.budget {
                    return Poll::Ready(Err(this.reject()));
                }
                this.remaining = length - 4;
                this.budget -= length + 1;
            }
        }
        if this.header_sent < 5 {
            let count = (5 - this.header_sent).min(out.remaining());
            out.put_slice(&this.header[this.header_sent..this.header_sent + count]);
            this.header_sent += count;
            if this.header_sent < 5 {
                return Poll::Ready(Ok(()));
            }
            // Return only the header in this poll. A partial read is always permitted.
        } else if this.remaining > 0 {
            let count = out.remaining().min(this.remaining);
            let mut limited = ReadBuf::new(out.initialize_unfilled_to(count));
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut limited))?;
            let received = limited.filled().len();
            if received == 0 {
                return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()));
            }
            out.advance(received);
            this.remaining -= received;
        }
        if this.remaining == 0 {
            this.header_read = 0;
            this.header_sent = 0;
        }
        Poll::Ready(Ok(()))
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for BoundedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    #[tokio::test]
    async fn oversized_frame_is_refused_before_any_header_or_body_reaches_driver() {
        let flag = Arc::new(AtomicBool::new(false));
        let malicious = [b'D', 0x7f, 0xff, 0xff, 0xff];
        let mut bounded = BoundedStream::new(malicious.as_slice(), 1024, 4096, flag.clone());
        let mut out = Vec::new();
        assert!(bounded.read_to_end(&mut out).await.is_err());
        assert!(out.is_empty());
        assert!(flag.load(Ordering::Relaxed));
    }
    #[tokio::test]
    async fn frames_survive_partial_reads_and_total_wire_budget_is_finite() {
        let frame = [b'Z', 0, 0, 0, 5, b'I'];
        let input = frame.repeat(3);
        let mut bounded =
            BoundedStream::new(input.as_slice(), 32, 12, Arc::new(AtomicBool::new(false)));
        let mut out = Vec::new();
        loop {
            let mut b = [0; 1];
            match bounded.read(&mut b).await {
                Ok(1) => out.push(b[0]),
                Err(_) => break,
                _ => panic!("expected budget rejection"),
            }
        }
        assert_eq!(out, frame.repeat(2));
    }
}
