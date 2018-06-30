/// Adaptors for saving large multipart fields to disk

use futures::{Poll, Future, Stream};

use tokio_fs::file::{File, CreateFuture};
use tokio_io::AsyncWrite;

use std::error::Error;
use std::path::PathBuf;
use std::{env, io};

use super::Multipart;
use super::field::FieldHeaders;
use super::field::{FoldFields, InitState};
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

    pub fn with_dir<P: Into<PathBuf>>(self, dir: P) -> SaveFields<S> {
        let dir = dir.into();
        let fold = self.multi.fold_field(
            InitField {
                dir,
            },

        );

        SaveFields {
            fold,
            entries: Entries::new(),
        }
    }

    pub fn temp(self) -> SaveFields<S> {
        self.with_dir(env::temp_dir())
    }
}

struct InitField {
    dir: PathBuf,
}

impl InitState for InitField {
    type StateFuture = InitFuture;

    fn init_state(&mut self, headers: FieldHeaders) -> InitFuture {
        let path = self.dir.join(::random_alphanumeric(12));

        InitFuture {
            create: File::create(path.clone()),
            field: Some((headers, path)),
        }
    }
}

struct InitFuture {
    create: CreateFuture<PathBuf>,
    field: Option<(FieldHeaders, PathBuf)>,
}

impl Future for InitFuture {
    type Item = State;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<State, io::Error> {
        try_macros!(self.field, None);

        let (field, path) = self.field.take().expect("`InitFuture` already yielded");
        let file = try_ready_ext!(self.create.poll(), Some((field, path)));

        ready(State {
            file,
            field,
            path,
        })
    }
}

struct State {
    file: File,
    field: FieldHeaders,
    path: PathBuf,
}

struct FoldChunkFuture<C> {
    chunk: C,
    read: usize,
    state: Option<State>,
}

impl<C: BodyChunk> Future for FoldChunkFuture<C> {
    type Item = State;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<State, io::Error> {
        try_macros!(self.state, None);

        let mut state = self.state.take().expect("`FoldChunkFuture` already yielded");

        loop {
            let slice = &self.chunk.as_slice()[self.read..];
            let read = try_ready_ext!(state.file.poll_write(slice), Some(state));

            if read == 0 {
                return ready(state);
            }

            self.read = self.read.checked_add(read).expect("FoldChunkFuture::poll(): overflow");
        }
    }
}

pub struct SaveFields<S: Stream> where S::Item: BodyChunk {
    fold: FoldFields<S, InitField, fn (S::Item, State) -> FoldChunkFuture<C>>,
    entries: Option<Entries>
}

impl<S: Stream> Future for SaveFields<S> {
    type Item = Entries;
    type Error = SaveError<S::Error>;

    fn poll(&mut self) -> Poll<Entries, SaveError<S::Error>> {
        {
            let mut entries = self.entries.as_mut().expect("`SaveFields` already yielded");
            while let Some(State { field, path, ..}) = try_ready!(self.fold.poll()) {

            }
        }

        ready(self.entries.take().expect("`SaveFields` already yielded"))
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
