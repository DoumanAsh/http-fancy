//! HTTP body utilities
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::{mem, task, fmt};

pub use http_body::{Frame, SizeHint};
pub use http_body::Body as HttpBody;

#[repr(transparent)]
///HTTP body
pub struct Body {
    inner: bytes::Bytes,
}

impl Body {
    ///Creates new instance
    pub const fn new(inner: bytes::Bytes) -> Self {
        Self {
            inner
        }
    }

    #[inline(always)]
    ///Creates empty body
    pub const fn empty() -> Self {
        Self::new(bytes::Bytes::new())
    }
}

impl From<Vec<u8>> for Body {
    #[inline(always)]
    fn from(buffer: Vec<u8>) -> Self {
        Self::new(buffer.into())
    }
}

impl From<&'static str> for Body {
    #[inline(always)]
    fn from(text: &'static str) -> Self {
        Self::new(bytes::Bytes::from_static(text.as_bytes()))
    }
}

impl From<String> for Body {
    #[inline(always)]
    fn from(text: String) -> Self {
        Self::new(text.into())
    }
}

impl HttpBody for Body {
    type Data = bytes::Bytes;
    type Error = core::convert::Infallible;

    #[inline(always)]
    fn poll_frame(mut self: Pin<&mut Self>, _cx: &mut task::Context<'_>) -> task::Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if !self.inner.is_empty() {
            let mut result = bytes::Bytes::new();
            mem::swap(&mut self.inner, &mut result);
            task::Poll::Ready(Some(Ok(Frame::data(result))))
        } else {
            task::Poll::Ready(None)
        }
    }

    #[inline(always)]
    fn is_end_stream(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline(always)]
    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.inner.len() as u64)
    }
}

impl fmt::Debug for Body {
    #[inline(always)]
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Body").field("len", &self.inner.len()).finish()
    }
}

///Possible errors from `Collector`
#[derive(Debug)]
pub enum CollectError<T, C> {
    ///Underlying error from Body
    Transport(T),
    ///Error accumulating body
    Collector(C),
    ///Body is over limit
    Overflow,
}

impl<T, C> CollectError<T, C> {
    #[cold]
    #[inline(never)]
    fn unlikely_collector(error: C) -> Self {
        Self::Collector(error)
    }
}

impl<T: fmt::Display, C: fmt::Display> fmt::Display for CollectError<T, C> {
    #[inline(always)]
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Transport(error) => fmt::Display::fmt(error, fmt),
            Self::Collector(error) => fmt::Display::fmt(error, fmt),
            Self::Overflow => fmt.write_str("Overflow"),
        }
    }
}

#[cfg(feature = "std")]
impl<T: fmt::Display + fmt::Debug, E: fmt::Display + fmt::Debug> std::error::Error for CollectError<T, E> {}

///`Collector` buffer
pub trait Collector: Unpin {
    ///Final result type
    type Output: Unpin;
    ///Potential error.
    ///
    ///If no error possible, just use `core::convert::Infallible`.
    type Error;

    ///Append data to `self`, returning `Some(error)` if it is impossible.
    fn append(&mut self, data: bytes::Bytes) -> Option<Self::Error>;

    ///Returns size of collected so far.
    fn len(&self) -> usize;

    ///Callback to be called when header map is encountered.
    fn on_trailers(&mut self, headers: http::HeaderMap);

    ///Callback to consume self, returning accumulated data.
    ///
    ///Only called once underlying body indicates it is consumed.
    ///But if user calls `Future` again despite contract, it is ok to return whatever shit you want
    fn consume(&mut self) -> Result<Self::Output, Self::Error>;
}

impl Collector for Vec<u8> {
    type Output = Vec<u8>;
    type Error = core::convert::Infallible;

    #[inline(always)]
    fn append(&mut self, data: bytes::Bytes) -> Option<Self::Error> {
        self.extend_from_slice(&data);
        None
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.len()
    }

    #[inline(always)]
    fn on_trailers(&mut self, _: http::HeaderMap) {
    }

    #[inline(always)]
    fn consume(&mut self) -> Result<Self::Output, Self::Error> {
        let mut result = Vec::new();
        mem::swap(&mut result, self);
        Ok(result)
    }
}

#[cfg(feature = "compress")]
enum DecompressState {
    Uninit(Vec<u8>),
    Plain(Vec<u8>),
    Zstd(zstd::stream::write::Decoder<'static, Vec<u8>>),
}

#[cfg(feature = "compress")]
///Smart body collector, that automatically de-compresses if it detects compression applied.
///
///Supported algorithms:
///- `zstd`
pub struct DecompressCollector {
    state: DecompressState,
}

#[cfg(feature = "compress")]
impl DecompressCollector {
    const ZSTD_HEADER: [u8; 4] = 0xFD2FB528u32.to_le_bytes();

    #[inline(always)]
    ///Creates new instance
    pub const fn new() -> Self {
        Self {
            state: DecompressState::Uninit(Vec::new())
        }
    }
}

#[cfg(feature = "compress")]
#[derive(Debug)]
///Decompression error
pub enum DecompressError {
    ///Zstd algorithm fail
    Zstd(std::io::Error)
}

#[cfg(feature = "compress")]
impl fmt::Display for DecompressError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Zstd(error) => fmt.write_fmt(format_args!("Zstd({})", error)),
        }
    }
}

#[cfg(feature = "compress")]
impl Collector for DecompressCollector {
    type Output = Vec<u8>;
    type Error = DecompressError;

    #[inline(always)]
    fn append(&mut self, data: bytes::Bytes) -> Option<Self::Error> {
        use std::io::Write;

        match &mut self.state {
            DecompressState::Uninit(ref mut buffer) => {
                buffer.extend_from_slice(&data);
                if buffer.len() < Self::ZSTD_HEADER.len() {
                    None
                } else {
                    if buffer.starts_with(&Self::ZSTD_HEADER) {
                        match zstd::stream::write::Decoder::new(Vec::new()) {
                            Ok(mut decoder) => match decoder.write_all(&buffer) {
                                Ok(()) => {
                                    self.state = DecompressState::Zstd(decoder);
                                    None
                                },
                                Err(error) => Some(DecompressError::Zstd(error)),
                            },
                            Err(error) => Some(DecompressError::Zstd(error)),
                        }
                    } else {
                        self.state = DecompressState::Plain(mem::take(buffer));
                        None
                    }
                }
            },
            DecompressState::Plain(ref mut buffer) => {
                buffer.extend_from_slice(&data);
                None
            },
            DecompressState::Zstd(ref mut decoder) => match decoder.write_all(&data) {
                Ok(()) => None,
                Err(error) => Some(DecompressError::Zstd(error)),
            },
        }
    }

    #[inline(always)]
    fn len(&self) -> usize {
        match &self.state {
            DecompressState::Uninit(buffer) => buffer.len(),
            DecompressState::Plain(buffer) => buffer.len(),
            DecompressState::Zstd(decoder) => decoder.get_ref().len(),
        }
    }

    #[inline(always)]
    fn on_trailers(&mut self, _: http::HeaderMap) {
    }

    #[inline(always)]
    fn consume(&mut self) -> Result<Self::Output, Self::Error> {
        use std::io::Write;

        let mut result = DecompressState::Uninit(Vec::new());
        mem::swap(&mut result, &mut self.state);
        match result {
            DecompressState::Uninit(result) => Ok(result),
            DecompressState::Plain(result) => Ok(result),
            DecompressState::Zstd(mut decoder) => match decoder.flush() {
                Ok(()) => Ok(decoder.into_inner()),
                Err(error) => Err(DecompressError::Zstd(error))
            }
        }
    }
}

///Future that collects `HttpBody`
///
///## Arguments
///
///- `T` - `HttpBody`
///- `C` - Collector that implements `Collector` interface
///- `S` - Size limit, when overflow happens, returns `Collect::Overflow` error
pub struct Collect<const S: usize, T, C> {
    body: T,
    collector: C,
}

impl<T, C, const S: usize> Collect<S, T, C> {
    ///Creates new instance
    pub fn new(body: T, collector: C) -> Self {
        Self {
            body,
            collector,
        }
    }
}

impl<E, T: HttpBody<Data = bytes::Bytes, Error = E> + Unpin, C: Collector, const S: usize> Future for Collect<S, T, C> {
    type Output = Result<C::Output, CollectError<E, C::Error>>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            let body = Pin::new(&mut this.body);
            match HttpBody::poll_frame(body, ctx) {
                task::Poll::Ready(Some(frame)) => match frame {
                    Ok(frame) => match frame.into_data() {
                        Ok(data) => match S.checked_sub(this.collector.len().saturating_add(data.len())) {
                            None => {
                                break task::Poll::Ready(Err(CollectError::Overflow))
                            }
                            Some(_) => match data.len() {
                                0 => continue,
                                _ => match this.collector.append(data) {
                                    Some(error) => break task::Poll::Ready(Err(CollectError::unlikely_collector(error))),
                                    None => continue,
                                }
                            },
                        },
                        Err(frame) => match frame.into_trailers() {
                            Ok(headers) => {
                                this.collector.on_trailers(headers);
                                continue;
                            },
                            Err(_) => unreach!(),
                        }
                    },
                    Err(error) => break task::Poll::Ready(Err(CollectError::Transport(error))),
                },
                task::Poll::Ready(None) => match this.collector.consume() {
                    Ok(result) => break task::Poll::Ready(Ok(result)),
                    Err(error) => break task::Poll::Ready(Err(CollectError::unlikely_collector(error))),
                },
                task::Poll::Pending => break task::Poll::Pending,
            };
        }
    }
}
