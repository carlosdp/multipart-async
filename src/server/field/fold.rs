
use futures::{IntoFuture, Future, Poll, Stream};
use futures::task::Context;

use std::mem;

use ::{BodyChunk, StreamError};
use helpers::*;
use server::Multipart;
use super::{FieldHeaders};

impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    /// Fold the body chunks of every field in the stream, producing a new stream which
    /// yields the folded state for each field.
    pub fn fold_fields<Fi, Fc>(self, init: Fi, fold: Fc) -> FoldFields<S, Fi, Fc>
        where Fi: InitState, Fc: FoldChunk<Fi::State, S::Item>  {
        FoldFields {
            multi: self,
            init,
            fold,
            state: FoldState::ReadHeaders,
        }
    }
}

pub trait InitState {
    type State;

    fn init_state(&mut self, headers: FieldHeaders) -> Self::State;
}

impl<St, F: FnMut(FieldHeaders) -> St> InitState for F {
    type State = St;

    fn init_state(&mut self, headers: FieldHeaders) -> <Self as InitState>::State {
        (self)(headers)
    }
}

pub trait FoldChunk<S, C: BodyChunk> {
    type Future: IntoFuture<Item = S>;

    fn fold_chunk(&mut self, state: S, chunk: C) -> Self::Future
    where <Self::Future as IntoFuture>::Error: StreamError;
}

impl<Fut, St, C: BodyChunk, F: FnMut(St, C) -> Fut> FoldChunk<St, C> for F
    where Fut: IntoFuture<Item = St> {
    type Future = Fut;

    fn fold_chunk(&mut self, state: St, chunk: C) -> <Self as FoldChunk<St, C>>::Future
        where <Self::Future as IntoFuture>::Error: StreamError {
        (self)(state, chunk)
    }
}

enum FoldState<St, Fut> {
    ReadHeaders,
    ReadyState(St),
    Future(Fut)
}

pub struct FoldFields<S: Stream, Fi: InitState, Fc: FoldChunk<Fi::State, S::Item>>
    where S::Item: BodyChunk {
    multi: Multipart<S>,
    init: Fi,
    fold: Fc,
    state: FoldState<Fi::State, <Fc::Future as IntoFuture>::Future>
}

impl<S: Stream, Fi, Fc> Stream for FoldFields<S, Fi, Fc>
    where S::Item: BodyChunk, S::Error: StreamError,
            Fi: InitState, Fc: FoldChunk<Fi::State, S::Item>,
          <Fc::Future as IntoFuture>::Error: StreamError + From<S::Error> {
    type Item = Fi::State;
    type Error = <Fc::Future as IntoFuture>::Error;

    fn poll_next(&mut self, ctxt: &mut Context) -> PollOpt<Self::Item, Self::Error> {
        use self::FoldState::*;

        try_macros!(self, ReadHeaders);

        loop {
            match mem::replace(&mut self.state, ReadHeaders) {
                ReadHeaders => {
                    let headers = try_ready_opt!(self.multi.read_headers(ctxt));
                    self.state = ReadyState(self.init.init_state(headers));
                },
                ReadyState(state) => {
                    let chunk = try_ready_opt!(self.multi.body_chunk(ctxt), ReadyState(state),
                                                 state);
                    self.state = Future(self.fold.fold_chunk(state, chunk).into_future());
                },
                Future(mut fut) => {
                    let state = try_ready_ext!(fut.poll(ctxt), Future(fut));
                    self.state = ReadyState(state);
                }
            }
        }

    }
}

