use futures_core::future::Future;
use futures_core::task::{Waker, Poll};
use futures_io::AsyncRead;
use std::io;
use std::pin::Pin;
use std::vec::Vec;

/// Future for the [`read_to_end`](super::AsyncReadExt::read_to_end) method.
#[derive(Debug)]
pub struct ReadToEnd<'a, R: ?Sized + Unpin> {
    reader: &'a mut R,
    buf: &'a mut Vec<u8>,
}

impl<R: ?Sized + Unpin> Unpin for ReadToEnd<'_, R> {}

impl<'a, R: AsyncRead + ?Sized + Unpin> ReadToEnd<'a, R> {
    pub(super) fn new(reader: &'a mut R, buf: &'a mut Vec<u8>) -> Self {
        ReadToEnd { reader, buf }
    }
}

struct Guard<'a> { buf: &'a mut Vec<u8>, len: usize }

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        unsafe { self.buf.set_len(self.len); }
    }
}

// This uses an adaptive system to extend the vector when it fills. We want to
// avoid paying to allocate and zero a huge chunk of memory if the reader only
// has 4 bytes while still making large reads if the reader does have a ton
// of data to return. Simply tacking on an extra DEFAULT_BUF_SIZE space every
// time is 4,500 times (!) slower than this if the reader has a very small
// amount of data to return.
//
// Because we're extending the buffer with uninitialized data for trusted
// readers, we need to make sure to truncate that if any of this panics.
fn read_to_end_internal<R: AsyncRead + ?Sized>(
    mut rd: Pin<&mut R>,
    waker: &Waker,
    buf: &mut Vec<u8>,
) -> Poll<io::Result<()>> {
    let mut g = Guard { len: buf.len(), buf };
    let ret;
    loop {
        if g.len == g.buf.len() {
            unsafe {
                g.buf.reserve(32);
                let capacity = g.buf.capacity();
                g.buf.set_len(capacity);
                rd.initializer().initialize(&mut g.buf[g.len..]);
            }
        }

        match rd.as_mut().poll_read(waker, &mut g.buf[g.len..]) {
            Poll::Ready(Ok(0)) => {
                ret = Poll::Ready(Ok(()));
                break;
            }
            Poll::Ready(Ok(n)) => g.len += n,
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => {
                ret = Poll::Ready(Err(e));
                break;
            }
        }
    }

    ret
}

impl<A> Future for ReadToEnd<'_, A>
    where A: AsyncRead + ?Sized + Unpin,
{
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, waker: &Waker) -> Poll<Self::Output> {
        let this = &mut *self;
        read_to_end_internal(Pin::new(&mut this.reader), waker, this.buf)
    }
}
