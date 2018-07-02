use futures::{stream, Async, Future, IntoFuture, Stream};

use super::{Multipart, FieldHeaders};
use helpers::*;
use {BodyChunk, StreamError};

impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    /// For each field in the stream, perform a fold operation over its body chunks,
    /// accumulating the folded state for each field and yielding each in the same order.
    ///
    /// The first closure, `init_state`, is called for each field with the field's headers.
    /// It should return a future which will yield the initial fold state for the field's
    /// body chunks. This is the optimum time to, say, open a file for writing the field's
    /// contents to.
    ///
    /// The second closure, `fold_chunk`, is called with the current fold state and the next
    /// body chunk for the current field. It should return a future which will fold the
    /// chunk into the field's current state and then yield the updated state.
    ///
    /// When all chunks of a field have been exhausted, the latest fold state is yielded by
    /// the stream returned from this method and `init_state` is called again, with the headers
    /// for the next field. This continues until the request is read to completion,
    /// or an error occurs.
    pub fn fold_fields<St, Is, Isf, Fc, Fcf, E>(mut self, mut init_state: Is, mut fold_chunk: Fc)
        -> impl Stream<Item = St, Error = E> where
            Is: FnMut(FieldHeaders) -> Isf,
            Isf: IntoFuture<Item = St, Error = E>,
            Fc: FnMut(St, S::Item) -> Fcf,
            Fcf: IntoFuture<Item = St, Error = E>,
            E: From<S::Error> {

        let mut init_fut: Option<Isf::Future> = None;
        let mut state: Option<St> = None;
        let mut fold_fut: Option<Fcf::Future> = None;

        stream::poll_fn(move || {
            loop {
                if let Some(mut init_fut_) = init_fut.take() {
                    try_macros!(init_fut, Some(init_fut_));
                    state = Some(try_ready_ext!(init_fut_.poll()));
                }

                if let Some(state_) = state.take() {
                    try_macros!(state, Some(state_));
                    let chunk = try_ready_opt!(self.poll_field_body(), None);
                    fold_fut = Some(fold_chunk(state_, chunk).into_future());
                }

                if let Some(mut fold_fut_) = fold_fut.take() {
                    try_macros!(fold_fut, Some(fold_fut_));
                    state = Some(try_ready_ext!(fold_fut_.poll()));
                }

                match self.poll_field_head()? {
                    Async::Ready(Some(headers)) =>
                        init_fut = Some(init_state(headers).into_future()),
                    Async::Ready(None) => return ready(None),
                    Async::NotReady => return not_ready(),
                }
            }
        }).fuse()
    }
}
