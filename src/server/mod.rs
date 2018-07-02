// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! The server-side abstraction for multipart requests. Enabled with the `server` feature (on by
//! default).
//!
//! Use this when you are implementing an HTTP server and want to
//! to accept, parse, and serve HTTP `multipart/form-data` requests (file uploads).
//!
//! See the `Multipart` struct for more info.
extern crate httparse;
extern crate twoway;

use futures::{Poll, Stream};
use futures::task::{self, Task};

use std::cell::Cell;
use std::rc::Rc;

use self::boundary::BoundaryFinder;

use {BodyChunk, StreamError};

macro_rules! try_opt (
    ($expr:expr) => (
        match $expr {
            Some(val) => val,
            None => return None,
        }
    )
);

macro_rules! ret_err (
    ($($args:tt)+) => (
            return fmt_err!($($args)+);
    )
);

macro_rules! fmt_err(
    ($string:expr) => (
        ::helpers::error($string)
    );
    ($string:expr, $($args:tt)*) => (
        ::helpers::error(format!($string, $($args)*))
    );
);

macro_rules! try_macros(
    ($statel:expr, $end:expr) => {
        macro_rules! try_ready_opt(
            ($try:expr) => (
                match $try {
                    Ok(Async::Ready(Some(val))) => val,
                    Ok(Async::Ready(None)) => {
                        $statel = $end;
                        return Ok(Async::Ready(None));
                    },
                    Ok(Async::NotReady) => {
                        return Ok(Async::NotReady);
                    },
                    Err(e) => return Err(e.into()),
                }
            );
            ($try:expr, $restore:expr) => (
                match $try {
                    Ok(Async::Ready(Some(val))) => val,
                    Ok(Async::Ready(None)) => {
                        $statel = $end;
                        return ready(None);
                    },
                    Ok(Async::NotReady) => {
                        $statel = $restore;
                        return Ok(Async::NotReady);
                    },
                    Err(e) => {
                        $statel = $restore;
                        return Err(e.into());
                    }
                }
            );
            ($try:expr, $restore:expr, $ret_end:expr) => (
                match $try {
                    Ok(Async::Ready(Some(val))) => val,
                    Ok(Async::Ready(None)) => {
                        $statel = $end;
                        return ready($ret_end);
                    },
                    Ok(Async::NotReady) => {
                        $statel = $restore;
                        return Ok(Async::NotReady);
                    },
                    Err(e) => {
                        $statel = $restore;
                        return Err(e.into());
                    }
                }
            )
        );

        macro_rules! try_ready_ext(
            ($try:expr) => (
                match $try {
                    Ok(Async::Ready(val)) => val,
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Err(err) => return Err(err.into()),
                }
            );
            ($try:expr, $restore:expr) => (
                match $try {
                    Ok(Async::Ready(val)) => val,
                    Ok(Async::NotReady) => {
                        $statel = $restore;
                        return Ok(Async::NotReady);
                    },
                    Err(e) => {
                        $statel = $restore;
                        return Err(e.into());
                    }
                }
            )
        );
    }
);

mod boundary;
mod field;
mod fold;

use helpers::*;

use self::field::ReadHeaders;

pub use self::field::{Field, FieldHeaders, FieldData, FoldText, ReadTextField, TextField};

#[cfg(feature = "hyper")]
mod hyper;

#[cfg(feature = "hyper")]
pub use self::hyper::{MinusBody, MultipartService};

#[cfg(feature = "save")]
pub mod save;

/// The entry point of server processing of `multipart/form-data` requests.
pub struct Multipart<S: Stream> {
    stream: BoundaryFinder<S>,
    read_hdr: ReadHeaders,
    consumed: bool,
}

// Q: why can't we just wrap up these bounds into a trait?
// A: https://github.com/rust-lang/rust/issues/24616#issuecomment-112065997
// (The workaround mentioned in a later comment doesn't seem to be worth the added complexity)
impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    /// Construct a new `Multipart` with the given body reader and boundary.
    ///
    /// This will add the requisite `--` and CRLF (`\r\n`) to the boundary as per
    /// [IETF RFC 7578 section 4.1](https://tools.ietf.org/html/rfc7578#section-4.1).
    pub fn with_body<B: Into<String>>(stream: S, boundary: B) -> Self {
        let mut boundary = boundary.into();
        boundary.insert_str(0, "--");

        debug!("Boundary: {}", boundary);

        Multipart {
            stream: BoundaryFinder::new(stream, boundary),
            read_hdr: ReadHeaders::default(),
            consumed: false,
        }
    }

    /// Process this request as a futures-idiomatic `Stream` of `Stream`s,
    /// where each substream yields chunks of a single field's contents in the request body.
    ///
    /// `MultipartStream` uses `Rc` to share the inner stream with the `Field`
    /// instances it yields, so this operation effectively locks the stream to a single
    /// thread for its lifetime. This also incurs a heap allocation and pointer indirection.
    pub fn into_stream(self) -> MultipartStream<S> {
        MultipartStream {
            inner: Rc::new(Inner::new(self))
        }
    }

    /// Low-level API: poll for the headers of the next field, discarding any remaining contents
    /// of the current one, if applicable.
    ///
    /// If `NotReady` is returned, this method may be called again when the task is woken without
    /// losing any data; it will continue attempting to read the same field's headers.
    /// If `None` is returned, the request body has been read to completion.
    pub fn poll_field_head(&mut self) -> PollOpt<FieldHeaders, S::Error> {
        // only attempt to consume the boundary if it hasn't been done yet
        self.consumed = self.consumed || try_ready!(self.stream.consume_boundary());

        if !self.consumed {
            return ready(None);
        }

        let res = try_ready!(self.read_hdr.read_headers(&mut self.stream));
        self.consumed = false;
        ready(res)
    }

    /// Low-level API: poll for the next chunk of the current field's body, or `None`
    /// if the field has been read to completion.
    pub fn poll_field_body(&mut self) -> PollOpt<S::Item, S::Error> {
        self.stream.body_chunk()
    }
}

/// A `multipart/form-data` request represented as a `Stream` of `Field`s.
///
/// This will parse the incoming stream into `Field` instances via its
/// `Stream` implementation.
///
/// As the underlying request body is a single contiguous stream of fields, this will not yield more
/// than one `Field` at a time; otherwise it would be necessary to buffer an entire field's
/// contents, which is ill-advised with multipart requests.
///
/// A `Drop` implementation on `FieldData` is used to signal when it's time to move forward, so do
/// avoid leaking that type or anything which contains it
/// (`Field`, `ReadTextField`, or any stream combinators).
///
/// Because this struct uses `Rc` internally, it is not `Send` or `Sync`.
pub struct MultipartStream<S: Stream> {
    inner: Rc<Inner<S>>,
}

/// As the underlying request is a single contiguous stream, this will only yield a single `Field`
/// instance at a time.
impl<S: Stream> Stream for MultipartStream<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = Field<S>;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        // to contain the borrow of `self.inner`
        let headers_res = Rc::get_mut(&mut self.inner)
            .map(|inner| inner.multi.get_mut().poll_field_head());

        match headers_res {
            // we were ready to move to the next field (`Field` was dropped)
            Some(res) => {
                match try_ready!(res) {
                    // headers were read
                    Some(headers) => {
                        info!("read field headers: {:?}", headers);
                        ready(field::new_field(headers, self.inner.clone()))
                    },
                    // end of stream
                    None => {
                        info!("stream at end");
                        ready(None)
                    }
                }
            },
            // a child `Field` still exists, we can't move forward
            None => {
                debug!("returning `NotReady`, field was in flight");
                self.inner.save_task();
                not_ready()
            }
        }
    }
}

struct Inner<S: Stream> {
    multi: Cell<Multipart<S>>,
    waiting: Cell<Option<Task>>,
}

impl<S: Stream> Inner<S> {
    fn new(multi: Multipart<S>) -> Self {
        Inner {
            multi: multi.into(),
            waiting: None.into(),
        }
    }

    fn save_task(&self) {
        self.waiting.set(Some(task::current()));
    }

    fn notify_task(&self) {
        self.waiting.take().map(|t| t.notify());
    }
}

/// An extension trait for requests which may be multipart.
pub trait RequestExt: Sized {
    /// The success type, may contain `Multipart` or something else.
    type Multipart;

    /// Convert `Self` into `Self::Multipart` if applicable.
    fn into_multipart(self) -> Result<Self::Multipart, Self>;
}
