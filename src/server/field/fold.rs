
use futures::{Future, Poll, Stream};
use futures::task::Context;

use std::mem;

use ::{BodyChunk, StreamError};
use helpers::*;
use server::Multipart;
use super::{FieldHeaders};

impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    pub fn poll_fields<St, F>(self, folder: F) -> FoldFields<S, F> where F: FieldFolder<S::Item> {
        FoldFields {
            multi: self,
            folder,
            state: FoldState::ReadHeaders,
        }
    }
}

pub trait FieldFolder<C: BodyChunk> {
    type State;
    type Future: Future<Item = Self::State>;

    fn init_state(&mut self, field: FieldHeaders) -> Self::State;

    fn fold_chunk(&mut self, state: Self::State, field_chunk: C) -> Self::Future
    where <Self::Future as Future>::Error: StreamError;
}

pub fn field_folder<C: BodyChunk, Fi, Fc, Fut, S>(init_state: Fi, fold_chunk: Fc)
    -> impl FieldFolder<C, State = S, Future = Fut>
where Fi: FnMut(FieldHeaders) -> S, Fc: FnMut(S, C) -> Fut, Fut: Future<Item = S> {
    struct FieldFolderImpl<Fi, Fc> {
        init: Fi,
        fold: Fc
    }

    impl<C, Fi, Fc, Fut, S> FieldFolder<C> for FieldFolderImpl<Fi, Fc>
    where Fi: FnMut(FieldHeaders) -> S, Fc: FnMut(S, C) -> Fut, Fut: Future<Item = S>  {
        type State = S;
        type Future = Fut;

        fn init_state(&mut self, field: FieldHeaders) -> <Self as FieldFolder<C>>::State {
            (self.init)(field)
        }

        fn fold_chunk(&mut self, state: <Self as FieldFolder<C>>::State, field_chunk: C)
            -> Self::Future where <Fut as Future>::Error: StreamError {
            (self.fold)(state, field_chunk)
        }
    }

    FieldFolderImpl {
        init: init_state,
        fold: fold_chunk,
    }
}

enum FoldState<St, Fut> {
    ReadHeaders,
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
    type Error = <F::Future as Future>::Error;

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
                    self.state = Future(self.folder.fold_chunk(state, chunk));
                },
                Future(mut fut) => {
                    let state = try_ready_ext!(fut.poll(ctxt), Future(fut));
                    self.state = ReadyState(state);
                }
            }
        }

    }
}

