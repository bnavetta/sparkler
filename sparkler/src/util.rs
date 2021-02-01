//! Miscellaneous utilities

use std::os::unix::io::AsRawFd;
use std::{fmt::Debug, fs::File};

use nix::{
    fcntl,
    mount::{mount, MsFlags},
};

use crate::error::Error;

/// For use with `mount`, to provide type annotations for `None`
const NONE: Option<&'static [u8]> = None;

/// RAII wrapper around an exclusive advisory file lock. The lock is released when this `FileLock` is dropped. If unlocking fails, an error is printed on stderr.
///
/// It holds a reference to the locked file to ensure that the lock does not outlive the underlying file descriptor.
///
/// See the `flock(2)` man page.
#[derive(Debug)]
pub struct FileLock<'a>(&'a File);

impl<'a> FileLock<'a> {
    /// Take an exclusive lock on `file`, returning a `FileLock` guard.
    pub fn new(file: &'a File) -> Result<FileLock, nix::Error> {
        fcntl::flock(file.as_raw_fd(), fcntl::FlockArg::LockExclusive)?;
        Ok(FileLock(file))
    }
}

impl<'a> Drop for FileLock<'a> {
    fn drop(&mut self) {
        if let Err(err) = fcntl::flock(self.0.as_raw_fd(), fcntl::FlockArg::Unlock) {
            eprint!("Unlocking file {:?} failed: {}", self.0, err);
        }
    }
}

/// Create a bind mount of `target` at `source`.
pub fn bind_mount<P1: ?Sized + nix::NixPath + Debug, P2: ?Sized + nix::NixPath + Debug>(
    source: &P1,
    target: &P2,
) -> Result<(), Error> {
    bind_mount_flags(source, target, MsFlags::empty())
}

/// Create a bind mount of `target` at `source`, with the given `flags` in addition to [`MsFlags::MS_BIND`].
pub fn bind_mount_flags<P1: ?Sized + nix::NixPath + Debug, P2: ?Sized + nix::NixPath + Debug>(
    source: &P1,
    target: &P2,
    flags: MsFlags,
) -> Result<(), Error> {
    mount(Some(source), target, NONE, flags | MsFlags::MS_BIND, NONE).map_err(|error| {
        Error::System {
            context: format!("could not bind {:?} to {:?}", source, target),
            error,
        }
    })
}
