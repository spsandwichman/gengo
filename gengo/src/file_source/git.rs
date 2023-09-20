use super::FileSource;
use gix::{
    attrs::StateRef,
    bstr::{BString, ByteSlice},
    Repository, ThreadSafeRepository,
    prelude::FindExt,
    discover::Error as DiscoverError,
};
use crate::{Error, ErrorKind};
use std::path::Path;
use std::borrow::Cow;
use std::error::Error as ErrorTrait;

pub struct Git {
    repository: ThreadSafeRepository,
    rev: String,
}

impl Git {
    pub fn new<P: AsRef<Path>>(path: P, rev: &str) -> Result<Self, Box<dyn ErrorTrait>> {
        let repository = match gix::discover(path) {
            Ok(r) => r,
            Err(DiscoverError::Discover(err)) => {
                return Err(Box::new(Error::with_source(ErrorKind::NoRepository, err)))
            }
            Err(err) => return Err(err.into()),
        };

        let repository = repository.into_sync();
        let rev = rev.to_string();
        Ok(Self { repository, rev })
    }
}

impl<'repo> FileSource<'repo> for Git {
    type Filepath = Cow<'repo, Path>;
    type Contents = &'repo [u8];
    type Iter = Iter<'repo>;

    fn files(&self) -> Self::Iter {
        todo!("Initialize iterator");
        Iter {
            state: (),
            stack: (),
            foo: & (),
        }
    }
}

pub struct Iter<'repo> {
    state: (),
    stack: (),
    foo: &'repo (),
}

impl<'repo> Iterator for Iter<'repo> {
    type Item = (Cow<'repo, Path>, &'repo [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        todo!("implement iteration");
    }
}