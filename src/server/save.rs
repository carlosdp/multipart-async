/// Adaptors for saving large multipart fields to disk

use futures::{Poll, Future, Stream, AsyncSink, Sink};
use futures::future::Either;

use tokio_fs::file::{File, CreateFuture};
use tokio_io::AsyncWrite;
use tokio_threadpool::blocking;

use std::collections::BTreeMap;
use std::error::Error;
use std::path::PathBuf;
use std::rc::Rc;
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
            let Field { headers, data, .. } = field;

            if headers.is_text() {
                Either::A(data.read_text())
            } else {
                let path = dir.join(::random_alphanumeric(12));
                Either::B(File::create(path.clone()).and_then(move |file| {
                    data.forward(FileSink { chunk: None, file })
                        .map(move |_| path)
                        .map_err(Into::into)
                }))
            }.then(move |res| match res {
                Ok(Either::A(string)) => {

                },
                Ok(Either::B(path)) => {

                },
                Err(e) => {
                    return Err(SaveError {
                        partial: entries,
                        field: Some(headers),
                        cause: e,
                    })
                }
            })
        })
    }

    pub fn temp(self) -> impl Future<Item = Entries, Error = SaveError<S::Error>> {
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

    fn start_send(&mut self, mut item: C) -> Result<AsyncSink<C>, io::Error> {
        if self.chunk.is_some() {
            return AsyncSink::NotReady(item);
        }

        while let Async::Ready(()) = self.poll_complete()? {}

        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Result<Async<()>, <Self as Sink>::SinkError> {
        try_macros!(self.chunk, None);

        let mut item = self.item.take().expect("`poll_complete()` called before `start_send()`");

        while !item.as_slice().is_empty() {
            let read = try_ready_ext!(self.file.poll_write(item.as_slice()), Some(chunk));
            item = item.split_at(read).1;
        }

        ready(())
    }

    fn close(&mut self) -> Result<Async<()>, <Self as Sink>::SinkError> {
        unimplemented!()
    }
}

pub struct SaveError<E> {
    pub partial: Entries,
    pub field: Option<Rc<FieldHeaders>>,
    pub cause: E,
}

pub enum FieldData {
    Text(String),
    File(PathBuf),
}

pub struct Entries {
    pub fields: BTreeMap<FieldHeaders, FieldData>,
    pub dir: PathBuf,
    keep_dir: bool,
    _priv: (),
}

impl Entries {
    fn new(dir: PathBuf) -> Entries {
        Entries {
            fields: BTreeMap::new(),
            dir,
            keep_dir: false,
            _priv: (),
        }
    }

    /// Get an iterator of all fields under a given name.
    ///
    /// Multipart requests are allowed to contain multiple fields with the same name;
    /// for example, a form field that can accept multiple files will be produce separate
    /// multipart entries with tbe same field name for each file
    pub fn fields_by_name(&self, field_name: &str) -> impl Iterator<Item = &(FieldHeaders, FieldData)> {
        self.fields.range(field_name ..= field_name)
    }

    pub fn keep_dir(&mut self) {
        self.keep_dir = true;
    }
}

impl Drop for Entries {
    fn drop(&mut self) {

    }
}