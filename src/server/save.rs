/// Adaptors for saving large multipart fields to disk

use futures::{Poll, Future, Stream, AsyncSink, Sink};
use futures::future::Either;

use tokio_fs::file::{File, CreateFuture};
use tokio_io::AsyncWrite;

use std::error::Error;
use std::path::PathBuf;
use std::{env, io};

use super::{Field, FieldHeaders, Multipart};
use helpers::*;
use BodyChunk;

impl<S: Stream> Multipart<S> {
    pub fn save(self) -> SaveBuilder<S> {
        SaveBuilder {
            multi: self,
            size_limit: 16 * 1024 * 1024,
            count_limit: 32,
        }
    }
}

pub struct SaveBuilder<S: Stream> {
    multi: Multipart<S>,
    size_limit: u64,
    count_limit: u64,
}

impl<S: Stream> SaveBuilder<S> {
    pub fn size_limit(self, size_limit: u64) -> Self {
        SaveBuilder { size_limit, ..self }
    }

    pub fn count_limit(self, count_limit: u64) -> Self {
        SaveBuilder { count_limit, ..self }
    }

    pub fn with_dir<P: Into<PathBuf>>(self, dir: P) -> impl Future<Item = Entries, Error = SaveError<S::Error>> {
        let dir = dir.into();

        self.multi.fold(Entries::new(), move |entries, field: Field<S>| {
            if field.headers.is_text() {
                Either::A(field.data.read_text())
            } else {
                let path = dir.join(::random_alphanumeric(12));
                Either::B(File::create(path.clone()).and_then(move |file| {
                    field.data.forward(FileSink { chunk: None, file }).map(move |_| path)
                }))
            }.then(move |res| match res {
                Ok(Either::A(string)) => {

                },
                Ok(Either::B(path)) => {

                }
            })
        })
    }

    pub fn temp(self) -> SaveFields<S> {
        self.with_dir(env::temp_dir())
    }
}

struct FileSink<C> {
    chunk: Option<C>,
    file: File
}

impl<C: BodyChunk> Sink for FileSink<C> {
    type SinkItem = C;
    type SinkError = io::Error;

    fn start_send(&mut self, item: C) -> Result<AsyncSink<C>, io::Error> {
        if self.chunk.is_some() {
            return AsyncSink::NotReady(item);
        }


    }

    fn poll_complete(&mut self) -> Result<Async<()>, <Self as Sink>::SinkError> {
        unimplemented!()
    }

    fn close(&mut self) -> Result<Async<()>, <Self as Sink>::SinkError> {
        unimplemented!()
    }
}

pub struct SaveError<E> {
    pub partial: Entries,
    pub field: Option<FieldHeaders>,
    pub cause: SaveErrorKind<E>
}

pub enum SaveErrorKind<E> {
    Io(io::Error),
    Stream(E),
}

pub struct Entries {

}
