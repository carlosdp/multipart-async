/// Adaptors for saving large multipart fields to disk

use futures::{Poll, Future, Stream};

use std::error::Error;
use std::path::PathBuf;
use std::{env, io};

use super::Multipart;
use super::field::FieldHeaders;

pub use tokio_fs::CreateFile;

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

    pub fn with_dir<P: Into<PathBuf>>(self, path: P) -> SaveFields<S> {
        SaveFields {
            multi: self.multi,
            path: path.info()
        }
    }

    pub fn temp(self) -> SaveFields<S> {
        self.with_dir(env::temp_dir())
    }
}

pub struct SaveFields<S: Stream> {
    multi: Multipart<S>,
    path: PathBuf,
}

impl<S: Stream> Future for SaveFields<S> {
    type Item = Entries;
    type Error = SaveError<S::Error>;

    fn poll(&mut self) -> Poll<Entries, SaveError<S::Error>> {

    }

    type
}

pub struct SaveError<E> {
    pub partial: Option<FieldHeaders>,
    cause:
}

pub enum SaveErrorKind<E> {
    Io(io::Error),
    Stream(E),
}