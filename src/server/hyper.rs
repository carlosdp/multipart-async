// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! Server-side integration with [Hyper](https://github.com/hyperium/hyper).
//! Enabled with the `hyper` feature (on by default).
use bytes::Bytes;

use futures::future::{Either, IntoFuture};

use hyper::header::CONTENT_TYPE;
use hyper::body::Payload;
pub use hyper::{Body, Chunk, Error, HeaderMap, Version, Method, Request, Response, Uri};
pub use hyper::service::Service;

use mime::{self, Mime};

use std::str::Utf8Error;

use super::{Multipart, MultipartStream, RequestExt};
use {BodyChunk, StreamError};

impl RequestExt for Request<Body> {
    type Multipart = (MultipartStream<Body>, MinusBody);

    fn into_multipart(self) -> Result<Self::Multipart, Self> {
        if let Some(boundary) = get_boundary(&self) {
            info!("multipart request received, boundary: {}", boundary);
            let (body, minus_body) = MinusBody::from_req(self);
            Ok((Multipart::with_body(body, boundary).into_stream(), minus_body))
        } else {
            Err(self)
        }
    }
}

/// A deconstructed `server::Request` with the body extracted.
#[allow(missing_docs)]
#[derive(Debug)]
pub struct MinusBody {
    pub method: Method,
    pub uri: Uri,
    pub version: Version,
    pub headers: HeaderMap,
}

impl MinusBody {
    fn from_req(req: Request<Body>) -> (Body, Self) {
        let (parts, body) = req.into_parts();
        (body, MinusBody {
            method: parts.method,
            uri: parts.uri,
            version: parts.version,
            headers: parts.headers
        })
    }
}

fn get_boundary(req: &Request<Body>) -> Option<String> {
    req.headers().get(CONTENT_TYPE)
        .and_then(|value| 
            match value.to_str() {
                Ok(v) => match v.parse::<mime::Mime>() {
                    Ok(ref m) => get_boundary_mime(m),
                    Err(_) => None,
                },
                Err(_) => None,
            }
        )
}

fn get_boundary_mime(mime: &Mime) -> Option<String> {
    if mime.type_() == mime::MULTIPART && mime.subtype() == mime::FORM_DATA {
        mime.get_param(mime::BOUNDARY).map(|n|n.as_ref().into())
    } else {
        None
    }
}

impl BodyChunk for Chunk {
    #[inline]
    fn split_at(self, idx: usize) -> (Self, Self) {
        let (first, second) = Bytes::from(self).split_at(idx);
        (first.into(), second.into())
    }

    #[inline]
    fn as_slice(&self) -> &[u8] {
        self
    }
}

impl StreamError for Error {
    fn from_str(str: &'static str) -> Self {
        unimplemented!()
    }

    fn from_string(string: String) -> Self {
        unimplemented!()
    }

    fn from_utf8(err: Utf8Error) -> Self {
        unimplemented!()
    }
}

use std::marker::PhantomData;

/// A `hyper::server::Service` implementation that handles extraction of a `Multipart` instance
pub struct MultipartService<M, N, MFut, NFut, Bd> {
    /// The handler for when the request is `multipart`
    pub multipart: M,
    /// The handler for all other requests
    pub normal: N,
    mfut: PhantomData<MFut>,
    nfut: PhantomData<NFut>,
    bd: PhantomData<Bd>,
}

impl<M, MFut, N, NFut, Bd> Service for MultipartService<M, N, MFut, NFut, Bd> where M: Fn(<Request<Bd> as RequestExt>::Multipart) -> MFut,
                                                                MFut: IntoFuture<Item = Response<Bd>, Error = Error>,
                                                                N: Fn(Request<Bd>) -> NFut,
                                                                NFut: IntoFuture<Item = Response<Bd>, Error = Error>,
                                                                Bd: Payload,
                                                                Request<Bd>: RequestExt {
    type ReqBody = Bd;
    type ResBody = Bd;
    type Error = Error;
    type Future = Either<MFut::Future, NFut::Future>;

    fn call(&mut self, req: Request<Bd>) -> Self::Future {
        match req.into_multipart() {
            Ok(multi) => Either::A((self.multipart)(multi).into_future()),
            Err(req) => Either::B((self.normal)(req).into_future()),
        }
    }
}
