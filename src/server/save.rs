/// Adaptors for saving large multipart fields to disk

use futures::{Poll, Future, Stream};
use futures::future::Either;

use tokio_fs::file::{File, CreateFuture};
use tokio_io::AsyncWrite;

use std::error::Error;
use std::path::PathBuf;
use std::{env, io};

use super::Multipart;
use super::field::FieldHeaders;
use super::field::{FoldFields, FoldText, InitState};
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
