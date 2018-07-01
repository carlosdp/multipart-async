// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures::{Stream, Poll};

use std::rc::Rc;
use std::str;

use server::{Inner, Multipart};

use std::fmt;

use {BodyChunk, StreamError};

mod text;
mod headers;

pub use self::headers::{FieldHeaders, ReadHeaders};

pub use self::text::{ReadTextField, TextField};

pub(super) fn new_field<S: Stream>(headers: FieldHeaders, internal: Rc<Inner<S>>) -> Field<S> {
    let headers = Rc::new(headers);

    Field {
        headers: headers.clone(),
        data: FieldData {
            headers,
            inner: internal
        },
        _priv: (),
    }
}

/// A single field in a multipart stream.
///
/// The data of the field is provided as a `Stream` impl in the `data` field.
///
/// To avoid the next field being initialized before this one is done being read
/// (in a linear stream), only one instance per `Multipart` instance is allowed at a time.
/// A `Drop` implementation on `FieldData` is used to notify `Multipart` that this field is done
/// being read, thus:
///
/// ### Warning About Leaks
/// If this value or the contained `FieldData` is leaked (via `mem::forget()` or some
/// other mechanism), then the parent `Multipart` will never be able to yield the next field in the
/// stream. The task waiting on the `Multipart` will also never be notified, which, depending on the
/// event loop/reactor/executor implementation, may cause a deadlock.
pub struct Field<S: Stream> {
    /// The headers of this field, including the name, filename, and `Content-Type`, if provided.
    pub headers: Rc<FieldHeaders>,
    /// The data of this field in the request, represented as a stream of chunks.
    pub data: FieldData<S>,
    _priv: (),
}

impl<S: Stream> fmt::Debug for Field<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Field")
            .field("headers", &self.headers)
            .field("data", &"<FieldData>")
            .finish()
    }
}

/// The data of a field in a multipart stream, as a stream of chunks.
///
/// It may be read to completion via the `Stream` impl, or collected to a string with `read_text()`.
///
/// To avoid the next field being initialized before this one is done being read
/// (in a linear stream), only one instance per `Multipart` instance is allowed at a time.
/// A `Drop` implementation on `FieldData` is used to notify `Multipart` that this field is done
/// being read, thus:
///
/// ### Warning About Leaks
/// If this value is leaked (via `mem::forget()` or some other mechanism), then the parent
/// `MultipartStream` will never be able to yield the next field in the stream. The task waiting on
/// the `MultipartStream` will also never be notified, which, depending on the
/// event loop/reactor/executor implementation, may cause a deadlock.
// N.B.: must **never** be Clone!
pub struct FieldData<S: Stream> {
    headers: Rc<FieldHeaders>,
    inner: Rc<Inner<S>>,
}

impl<S: Stream> FieldData<S> where S::Item: BodyChunk, S::Error: StreamError {
    /// Get a `Future` which attempts to read the field data to a string.
    ///
    /// If a field is meant to be read as text, it will either have no content-type or
    /// will have a content-type that starts with "text"; `FieldHeaders::is_text()` is
    /// provided to help determine this.
    ///
    /// A default length limit for the string, in bytes, is set to avoid potential DoS attacks from
    /// attackers running the server out of memory. If an incoming chunk is expected to push the
    /// string over this limit, an error is returned. The limit value can be inspected and changed
    /// on `ReadTextField` if desired.
    ///
    /// ### Charset
    /// For simplicity, the default UTF-8 character set is assumed, as defined in
    /// [IETF RFC 7578 Section 5.1.2](https://tools.ietf.org/html/rfc7578#section-5.1.2).
    /// If the field body cannot be decoded as UTF-8, an error is returned.
    ///
    /// Decoding text in a different charset (except ASCII which
    /// is compatible with UTF-8) is, currently, beyond the scope of this crate. However, as a
    /// convention, web browsers will send `multipart/form-data` requests in the same
    /// charset as that of the document (page or frame) containing the form, so if you only serve
    /// ASCII/UTF-8 pages then you won't have to worry too much about decoding strange charsets.
    pub fn read_text(self) -> ReadTextField<Self> {
        if !self.headers.is_text() {
            debug!("attempting to read a non-text field as text: {:?}", self.headers);
        }

        text::read_text(self.headers.clone(), self)
    }

    fn inner_mut(&mut self) -> &mut Multipart<S> {
        assert!(Rc::strong_count(&self.inner) <= 2,
                "More than two copies of an `Rc<Internal>` at one time");

        // This is safe as we have guaranteed exclusive access, the lifetime is tied to `self`,
        // and is never null.
        unsafe { &mut *self.inner.multi.as_ptr() }
    }
}

impl<S: Stream> Stream for FieldData<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.inner_mut().poll_field_body()
    }
}

/// Notifies a task waiting on the parent `Multipart` that another field is available.
///
/// ### Do Not Leak!
impl<S: Stream> Drop for FieldData<S> {
    fn drop(&mut self) {
        self.inner.notify_task();
    }
}
