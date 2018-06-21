// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use std::borrow::Cow;
use std::{fmt, mem};
use std::str::Utf8Error;

use {BodyChunk, StreamError};


pub use futures::{Poll, Async};

pub type PollOpt<T, E> = Poll<Option<T>, E>;

pub fn ready<R, E, T: Into<R>>(val: T) -> Poll<R, E> {
    Ok(Async::Ready(val.into()))
}

pub fn not_ready<T, E>() -> Poll<T, E> {
    Ok(Async::NotReady)
}

pub fn error<T, E: Into<Cow<'static, str>>, E_: StreamError>(e: E) -> Result<T, E_> {
    Err(match e.into() {
        Cow::Owned(string) => E_::from_string(string),
        Cow::Borrowed(str) => E_::from_str(str),
    })
}

pub fn utf8_err<T, E: StreamError>(e: Utf8Error) -> Result<T, E> {
    Err(E::from_utf8(e))
}

pub fn replace_default<T: Default>(dest: &mut T) -> T {
    mem::replace(dest, T::default())
}

pub fn poll_into_opt<T, E>(poll: Poll<T, E>) -> PollOpt<T, E> {
    poll.map(|ok| ok.map(Some))
}

pub fn show_bytes<'a>(bytes: &'a [u8]) -> impl fmt::Display + 'a {
    ::display_bytes::HEX_UTF8.print_terminators(true).display_bytes(bytes)
}
