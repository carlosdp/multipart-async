
use futures::{IntoFuture, Future, Poll, Stream};
use futures::task::Context;

use std::mem;

use ::{BodyChunk, StreamError};
use helpers::*;
use server::Multipart;
use super::{FieldHeaders};

impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    pub fn fold_fields<St, F>(self, folder: F) -> FoldFields<S, F> where F: FieldFolder<S::Item> {
        FoldFields {
            multi: self,
            folder,
            state: FoldState::ReadHeaders,
        }
    }
}

pub trait FieldFolder<C: BodyChunk> {
    type State;
    type IntoFuture: IntoFuture<Item = Self::State>;

    fn init_state(&mut self, field: FieldHeaders) -> Self::State;

    fn fold_chunk(&mut self, state: Self::State, field_chunk: C) -> Self::IntoFuture
    where <Self::IntoFuture as IntoFuture>::Error: StreamError;
}


enum FoldState<St, Fut> {
    ReadHeaders,
    ReadyState(St),
    Future(Fut)
}

pub struct FoldFields<S: Stream, F: FieldFolder<S::Item>> where S::Item: BodyChunk {
    multi: Multipart<S>,
    folder: F,
    state: FoldState<F::State, <F::IntoFuture as IntoFuture>::Future>
}

impl<S: Stream, F: FieldFolder<S::Item>> Stream for FoldFields<S, F>
    where S::Item: BodyChunk, S::Error: StreamError {
    type Item = F::State;
    type Error = <F::IntoFuture as IntoFuture>::Error;

    fn poll_next(&mut self, ctxt: &mut Context) -> PollOpt<Self::Item, Self::Error> {
        use self::FoldState::*;

        try_macros!(self, ReadHeaders);

        loop {
            match mem::replace(&mut self.state, ReadHeaders) {
                ReadHeaders => {
                    let headers = try_ready_opt!(self.multi.read_headers(ctxt));
                    self.state = ReadyState(self.folder.init_state(headers));
                },
                ReadyState(state) => {
                    let chunk = try_ready_opt!(self.multi.body_chunk(ctxt), ReadyState(state),
                                                 state);
                    self.state = Future(self.folder.fold_chunk(state, chunk).into_future());
                },
                Future(mut fut) => {
                    let state = try_ready_ext!(fut.poll(ctxt), Future(fut));
                    self.state = ReadyState(state);
                }
            }
        }

    }
}

