
use futures::{Future, Poll, Stream};
use futures::task::Context;

use std::mem;

use ::{BodyChunk, StreamError};
use server::Multipart;
use super::{FieldHeaders};

impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    pub fn poll_fields<St, F>(self, folder: F) -> FoldFields<S, F>
    where F: FieldFolder<S::Item>  {
        FoldFields {
            multi: self,
            folder,
            state: FoldState::Empty,
        }
    }
}

pub trait FieldFolder<C: BodyChunk> {
    type State;
    type Future: Future<Item = Self::State>;

    fn init(&mut self, field: FieldHeaders) -> Self::State;

    fn fold_field(&mut self, state: Self::State, field_chunk: C) -> Self::Future
    where Self::Future::Error: StreamError;
}

enum FoldState<St, Fut> {
    Empty,
    ReadyState(St),
    Future(Fut)
}

pub struct FoldFields<S: Stream, F: FieldFolder<S::Item>> {
    multi: Multipart<S>,
    folder: F,
    state: FoldState<F::State, F::Future>
}

impl<S: Stream, F: FieldFolder<S::Item>> Stream for FoldFields<S, F> {
    type Item = F::State;
    type Error = F::Future::Error;

    fn poll_next(&mut self, ctxt: &mut Context) -> Poll<Option<<Self as Stream>::Item>, <Self as Stream>::Error> {
        use self::FoldState::*;

        macro_rules! try_ready_opt(
            ($try:expr) => (
                match $try {
                    Ok(Async::Ready(Some(val))) => val,
                    Ok(Async::Ready(None)) => {
                        self.state = End;
                        return ready(None);
                    }
                    other => return other.into(),
                }
            );
            ($try:expr; $restore:expr) => (
                match $try {
                    Ok(Async::Ready(Some(val))) => val,
                    Ok(Async::Ready(None)) => {
                        self.state = End;
                        return ready(None);
                    },
                    other => {
                        self.state = $restore;
                        return other.into();
                    }
                }
            )
        );

        loop {
            match mem::replace(&mut self.state, Empty) {
                Empty => {
                    let headers = try_ready_opt!(self, self.multi.read_headers(ctxt), Empty);
                    self.state = ReadyState(self.folder.init(headers));
                },
            }
        }

    }
}

