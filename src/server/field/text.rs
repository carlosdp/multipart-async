use futures::{Future, Stream};
use futures::Async::*;

use std::rc::Rc;
use std::{fmt, mem, str};

use {BodyChunk, StreamError};

use super::FieldHeaders;

use helpers::*;

#[derive(Debug)]
pub struct FoldText<C> {
    accum: String,
    prev_chunk: Option<C>,
    limit: usize,
}



/// The result of reading a `Field` to text.
#[derive(Clone, Debug)]
pub struct TextField {
    /// The headers for the original field, provided as a convenience.
    pub headers: Rc<FieldHeaders>,
    /// The text of the field.
    pub text: String,
}

/// A `Future` which attempts to read a field's data to a string.
///
/// ### Charset
/// For simplicity, the default UTF-8 character set is assumed, as defined in
/// [IETF RFC 7578 Section 5.1.2](https://tools.ietf.org/html/rfc7578#section-5.1.2).
/// If the field body cannot be decoded as UTF-8, an error is returned.
///
/// Decoding text in a different charset (except ASCII which is compatible with UTF-8) is,
/// currently, beyond the scope of this crate. However, as a convention, web browsers will send
/// `multipart/form-data` requests in the same charset as that of the document (page or frame)
/// containing the form, so if you only serve ASCII/UTF-8 pages then you won't have to worry
/// too much about decoding strange charsets.
///
/// ### Warning About Leaks
/// If this value or the contained `FieldData` is leaked (via `mem::forget()` or some
/// other mechanism), then the parent `Multipart` will never be able to yield the next field in the
/// stream. The task waiting on the `Multipart` will also never be notified, which, depending on the
/// event loop/reactor/executor implementation, may cause a deadlock.
#[derive(Debug)]
pub struct ReadTextField<S: Stream> {
    stream: Option<S>,
    accum: String,
    prev_chunk: Option<S::Item>,
    limit: usize,
    headers: Rc<FieldHeaders>,
}

// RFC on these numbers, they're pretty much arbitrary
const DEFAULT_LIMIT: usize = 65536; // 65KiB--reasonable enough for one text field, right?
const SOFT_MAX_LIMIT: usize = 16_777_216; // 16MiB--highest sane value for one text field, IMO

pub fn read_text<S: Stream>(headers: Rc<FieldHeaders>, data: S) -> ReadTextField<S> {
    ReadTextField {
        stream: Some(data),
        headers,
        accum: String::new(),
        prev_chunk: None,
        limit: DEFAULT_LIMIT,
    }
}

impl<S: Stream> ReadTextField<S> {
    /// Set the length limit, in bytes, for the collected text. If an incoming chunk is expected to
    /// push the string over this limit, an error is returned and the offending chunk is saved
    /// separately in the struct.
    ///
    /// Setting a value higher than a few megabytes is not recommended as it could allow an attacker
    /// to DoS the server by running it out of memory, causing it to panic on allocation or spend
    /// forever swapping pagefiles to disk. Remember that this limit is only for a single field
    /// as well.
    ///
    /// Setting this to `usize::MAX` is equivalent to removing the limit as the string
    /// would overflow its capacity value anyway.
    pub fn set_limit(self, limit: usize) -> Self {
        Self { fold_text: self.fold_text.set_limit(limit), .. self}
    }

    /// Soft max limit if the default isn't large enough.
    ///
    /// Going higher than this is allowed, but not recommended.
    pub fn limit_soft_max(self) -> Self {
        Self { fold_text: self.fold_text.limit_soft_max(), .. self}
    }

    /// The text that has been collected so far.
    pub fn ref_text(&self) -> &str {
        self.fold_text.ref_text()
    }

    /// Destructure this future, taking the internal `FieldData` instance back.
    ///
    /// Will be `None` if the field was read to completion, because the internal `FieldData`
    /// instance is dropped afterwards to allow the parent `Multipart` to immediately start
    /// working on the next field.
    pub fn into_data(self) -> Option<S> {
        self.stream
    }

    pub fn try_take_string<E: StreamError>(&mut self) -> Result<String, E> {
        if let Some(ref prev_chunk) = self.prev_chunk {
            ret_err!("stream terminated with incomplete UTF-8 sequence: {:?}",
                     prev_chunk.as_slice());
        }

        Ok(mem::replace(&mut self.accum, String::new()))
    }
}

impl<S: Stream> Future for ReadTextField<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = TextField;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Self::Item, S::Error> {
        loop {
            let mut stream = self.stream.as_mut()
                .expect("`ReadTextField::poll()` called again after yielding value");

            let mut next_chunk = match try_ready!(stream.poll()) {
                Some(val) => val,
                _ => break,
            };

            if let Some(prev_chunk) = self.prev_chunk.take() {
                // recombine the cutoff UTF-8 sequence
                let char_width = utf8_char_width(prev_chunk.as_slice()[0]);
                let needed_len =  char_width - prev_chunk.len();

                if next_chunk.len() < needed_len {
                    ret_err!("got a chunk smaller than the {} byte(s) needed to finish \
                          decoding this UTF-8 sequence: {:?}",
                         needed_len, prev_chunk.as_slice());
                }

                let over_limit = self.accum.len().checked_add(prev_chunk.len())
                    .and_then(|len| len.checked_add(next_chunk.len()))
                    .map_or(true, |len| len > self.limit);

                if over_limit {
                    ret_err!("text field exceeded limit of {} bytes", self.limit);
                }

                let mut buf = [0u8; 4];

                // first.len() will be between 1 and 4 as guaranteed by `Utf8Error::valid_up_to()`
                buf[..prev_chunk.len()].copy_from_slice(prev_chunk.as_slice());
                buf[prev_chunk.len()..].copy_from_slice(&next_chunk.as_slice()[..needed_len]);

                // if this fails we definitely got an invalid byte sequence
                let s = str::from_utf8(&buf[..char_width]).or_else(utf8_err::<_, E>)?;
                self.accum.push_str(s);

                next_chunk = next_chunk.split_at(needed_len).1;
            }

            // this also catches capacity overflows
            if self.accum.len().checked_add(next_chunk.len()).map_or(true, |len| len > self.limit) {
                ret_err!("text field exceeded limit of {} bytes", self.limit);
            }

            // try to convert the chunk to UTF-8 and append it to the accumulator
            let split_idx = match str::from_utf8(next_chunk.as_slice()) {
                Ok(s) => { self.accum.push_str(s); return Ok(()); },
                Err(e) => if e.valid_up_to() > next_chunk.len() - 4 {
                    // this may just be a valid sequence split across two chunks
                    e.valid_up_to()
                } else {
                    // definitely was an invalid byte sequence
                    return utf8_err(e);
                },
            };

            let (valid, invalid) = next_chunk.split_at(split_idx);

            // optimizer should be able to elide this second validation
            self.accum.push_str(str::from_utf8(valid.as_slice())
                .expect("a `BodyChunk` was UTF-8 before, now it's not"));

            self.prev_chunk = Some(invalid);
        }

        let text = self.try_take_string()?;

        // Optimization: free the `FieldData` so the parent `Multipart` can yield
        // the next field.
        self.stream = None;

        ready(TextField {
            headers: self.headers.clone(),
            text,
        })
    }
}

// Below lifted from https://github.com/rust-lang/rust/blob/1.19.0/src/libcore/str/mod.rs#L1461-L1485
// because they're being selfish with their UTF-8 implementation internals
static UTF8_CHAR_WIDTH: [u8; 256] = [
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, // 0x1F
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, // 0x3F
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, // 0x5F
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, // 0x7F
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, // 0x9F
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, // 0xBF
    0,0,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
    2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2, // 0xDF
    3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3, // 0xEF
    4,4,4,4,4,0,0,0,0,0,0,0,0,0,0,0, // 0xFF
];

#[inline]
fn utf8_char_width(b: u8) -> usize {
    return UTF8_CHAR_WIDTH[b as usize] as usize;
}
