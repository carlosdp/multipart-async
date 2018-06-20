use futures::{Stream, Poll};
use futures::task::{Context, LocalMap, Wake, Waker};

use std::borrow::{Borrow, Cow};
use std::collections::VecDeque;
use std::sync::Arc;
use std::{slice, io};

use helpers::*;
use StreamError;

#[macro_export]
macro_rules! mock_stream {
    ($($chunk:expr $(, $repeat:expr)*);*) => {{
        use $crate::mock::IntoPoll;

        $crate::mock::MockStream::new(
            &[$(($chunk.into_poll(), repeat_expr!($($repeat)*))),*]
        )
    }}
}

macro_rules! repeat_expr {
    () => { 0 };
    ($repeat:expr) => { $repeat }
}

pub struct MockStream {
    items: VecDeque<(Poll<Cow<'static, [u8]>, StringError>, u32)>,
}

impl MockStream {
    pub fn new<'a, I>(items: I) -> Self
        where I: IntoIterator<Item = &'a (Poll<Cow<'static, [u8]>, StringError>, u32)> {

        MockStream {
            items: items.into_iter().cloned().collect()
        }
    }
}

impl Stream for MockStream {
    type Item = Cow<'static, [u8]>;
    type Error = StringError;

    fn poll_next(&mut self, ctxt: &mut Context) -> PollOpt<Self::Item, Self::Error> {
        let (item, mut repeat) = if let Some(val) = self.items.pop_front() { val } else {
            return ready(None);
        };

        if repeat > 0 { self.items.push_front((item.clone(), repeat - 1)); }

        poll_into_opt(item)
    }
}

/// An adaptor trait to make `mock_stream!()` easier to use,
/// shouldn't be necessary for users to be aware of it.
#[doc(hidden)]
pub trait IntoPoll {
    fn into_poll(self) -> Poll<Cow<'static, [u8]>, StringError>;
}

impl IntoPoll for &'static [u8] {
    fn into_poll(self) -> Poll<Cow<'static, [u8]>, StringError> {
        Ok(Async::Ready(self.into()))
    }
}

impl IntoPoll for Vec<u8> {
    fn into_poll(self) -> Poll<Cow<'static, [u8]>, StringError> {
        Ok(Async::Ready(self.into()))
    }
}

impl IntoPoll for &'static str {
    fn into_poll(self) -> Poll<Cow<'static, [u8]>, StringError> {
        Ok(Async::Ready(self.as_bytes().into()))
    }
}

impl IntoPoll for String {
    fn into_poll(self) -> Poll<Cow<'static, [u8]>, StringError> {
        Ok(Async::Ready(self.into_bytes().into()))
    }
}

impl IntoPoll for Poll<Cow<'static, [u8]>, &'static str> {
    fn into_poll(self) -> Poll<Cow<'static, [u8]>, StringError> {
        self.map_err(|s| StringError(s.into()))
    }
}

impl IntoPoll for Poll<Cow<'static, [u8]>, String> {
    fn into_poll(self) -> Poll<Cow<'static, [u8]>, StringError> {
        self.map_err(|s| StringError(s.into()))
    }
}

/// A `StreamError` impl wrapping a string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StringError(String);

impl StreamError for StringError {
    fn from_str(str: &'static str) -> Self {
        StringError(str.into())
    }

    fn from_string(string: String) -> Self {
        StringError(string)
    }
}

impl Into<String> for StringError {
    fn into(self) -> String {
        self.0
    }
}

impl From<io::Error> for StringError {
    fn from(err: io::Error) -> Self {
        StringError(err.to_string())
    }
}

impl PartialEq<str> for StringError {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl<'a> PartialEq<&'a str> for StringError {
    fn eq(&self, other: &&'a str) -> bool {
        self.0 == *other
    }
}

#[doc(hidden)]
pub fn into_poll<T: IntoPoll>(from: T) -> Poll<Cow<'static, [u8]>, StringError> {
    from.into_poll()
}

pub fn into_poll_opt<T: IntoPoll>(from: T) -> PollOpt<Cow<'static, [u8]>, StringError> {
    from.into_poll().map(|ok| ok.map(Some))
}

pub fn with_context<F: FnOnce(&mut Context)>(closure: F) {
    struct DumbWaker;

    impl Wake for DumbWaker {
        fn wake(arc_self: &Arc<Self>) {}
    }

    let mut local_map = LocalMap::new();
    let waker = Waker::from(Arc::new(DumbWaker));

    let mut context = Context::without_spawn(&mut local_map, &waker);

    closure(&mut context)
}

#[cfg(test)]
mod test {
    use std::borrow::Cow;

    use helpers::*;
    use futures::Stream;

    use super::{into_poll, into_poll_opt, with_context};

    #[test]
    fn test_into_poll() {
        assert_eq!(
            Ok(Async::Ready(Cow::Borrowed(&b"Hello, world!"[..]))),
            into_poll("Hello, world!")
        );
    }

    #[test]
    fn test_empty_mock() {
        with_context(|ctxt| {
            assert_eq!(mock_stream!().poll_next(ctxt), ready(None));
        })
    }

    #[test]
    fn test_extra_poll() {
        with_context(|ctxt| {
            let mut stream = mock_stream!();
            assert_eq!(stream.poll_next(ctxt), ready(None));
            assert_eq!(stream.poll_next(ctxt), ready(None));
        });
    }

    #[test]
    fn test_yield_once() {
        with_context(|ctxt| {
            let mut stream = mock_stream!("Hello, world!");
            assert_eq!(stream.poll_next(ctxt), into_poll_opt("Hello, world!"));
            assert_eq!(stream.poll_next(ctxt), ready(None));
        });
    }

    #[test]
    fn test_repeat_once() {
        with_context(|ctxt| {
            let mut stream = mock_stream!("Hello, world!", 1);
            assert_eq!(stream.poll_next(ctxt), into_poll_opt("Hello, world!"));
            assert_eq!(stream.poll_next(ctxt), ready(Some(b"Hello, world!".as_ref().into())));
            assert_eq!(stream.poll_next(ctxt), ready(None));
        });
    }

    #[test]
    fn test_two_items() {
        with_context(|ctxt| {
            let mut stream = mock_stream!("Hello, world!"; "Hello, also!");
            assert_eq!(stream.poll_next(ctxt), into_poll_opt("Hello, world!"));
            assert_eq!(stream.poll_next(ctxt), into_poll_opt("Hello, also!"));
            assert_eq!(stream.poll_next(ctxt), ready(None));
        });
    }

    #[test]
    fn test_two_items_one_repeat() {
        with_context(|ctxt| {
            let mut stream = mock_stream!("Hello, world!", 1; "Hello, also!");
            assert_eq!(stream.poll_next(ctxt), into_poll_opt("Hello, world!"));
            assert_eq!(stream.poll_next(ctxt), into_poll_opt("Hello, world!"));
            assert_eq!(stream.poll_next(ctxt), into_poll_opt("Hello, also!"));
            assert_eq!(stream.poll_next(ctxt), ready(None));
        });
    }
}
