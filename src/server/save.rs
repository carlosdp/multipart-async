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

use super::{Field, FieldHeaders, FoldText, Multipart};
use helpers::*;
use {BodyChunk, StreamError};

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

impl<S: Stream> SaveBuilder<S> where S::Item: BodyChunk, S::Error: StreamError {
    pub fn size_limit(self, size_limit: u64) -> Self {
        SaveBuilder { size_limit, ..self }
    }

    pub fn count_limit(self, count_limit: u64) -> Self {
        SaveBuilder { count_limit, ..self }
    }

    pub fn with_dir<P: Into<PathBuf>>(self, dir: P) -> impl Future<Item = Entries, Error = SaveError<S::Error>> {
        let dir = dir.into();
        let dir_ = dir.clone();
        self.multi.fold_fields(
            move |headers| {
                if headers.is_text() {
                    Either::A(Ok((headers, FoldText::new())).into_future())
                } else {
                    let path = dir_.join(::random_alphanumeric(12));
                    let path_ = path.clone();
                    Either::B(File::create(path).and_then(move |file| (file, path_)))
                }
            },
            |state, chunk| {
                match state {
                    Either::A((header, fold_text)) => Either::A(match fold_text.fold_chunk(chunk) {
                        Ok(()) => Ok((header, fold_text)),
                        Err(e) => Err((header, e)),
                    }.into_future()),
                    Either::B(state) => Either::B(
                        FileWriteFuture {
                            chunk, start: 0, state: Some(state)
                        }
                    )
                }
            }
        )
    }

    pub fn temp(self) -> impl Future<Item = Entries, Error = SaveError<S::Error>> {
        self.with_dir(env::temp_dir())
    }
}

struct FileState {
    headers: FieldHeaders,
    file: File,
    path: PathBuf,
}

struct FileWriteFuture<C> {
    chunk: C,
    start: usize,
    state: Option<FileState>,
}

impl<C: BodyChunk> Future for FileWriteFuture<C> {
    type Item = FileState;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<FileState, io::Error> {
        while start < self.chunk.len() {
            let mut state = self.state.as_mut().expect("`FileWriteFuture` already yielded");
            let slice = &self.chunk.as_slice()[start..];
            let read = try_ready!(state.file.poll_write(slice));
            self.start = self.start.checked_add(read).expect("overflow, FileWriteFuture::poll");
        }

        ready(self.state.take().expect("`FileWriteFuture` already yielded"))
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
