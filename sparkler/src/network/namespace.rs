//! Utilities for dealing with Linux network namespaces

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

use nix::{
    errno::Errno,
    mount::{mount, umount2, MntFlags, MsFlags},
    sched,
    sys::stat::Mode,
};

use crate::util::{bind_mount, bind_mount_flags, FileLock};
use crate::Error;

// This uses the same approach as
// - https://git.kernel.org/pub/scm/network/iproute2/iproute2.git/tree/ip/ipnetns.c
// - https://github.com/containernetworking/plugins/blob/master/pkg/testutils/netns_linux.go
//
// We save the current network namespace, create a new one with unshare(2), bind-mount it to a persistent path, and then restore the original namespace.
// Using unshare(2) avoids the overhead of a clone(2), and the bind mount ensures that the namespace sticks around even with no processes using it.

/// Persistent network namespaces are (at least by convention) bound to files under /var/run/netns.
const NETNS_RUNTIME_DIRECTORY: &str = "/var/run/netns";

/// For use with `mount`, to provide type annotations for `None`
const NONE: Option<&'static [u8]> = None;

/// RAII guard for restoring a network namespace. When this is dropped, it switches back to the network namespace using [`sched::setns`]. If this fails, the implementation
// panics because we cannot meaningfully recover from being in the wrong network namespace.
struct NamespaceGuard(File);

/// Create a persistent network namespace named `name`.
pub fn create(name: &str) -> Result<PathBuf, Error> {
    prepare_runtime_directory()?;

    let namespace_path = persistent_namespace_path(name);

    // Step 1: Create the file for the network namespace (so we later have a file to bind-mount to)
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&namespace_path)
        .map_err(|error| Error::Io {
            context: format!(
                "could not create network namespace file {}",
                namespace_path.display()
            ),
            error,
        })?;

    // Step 2.0: Save our current network namespace
    let _guard = NamespaceGuard::from_current()?;

    // Step 2: Create a new network namespace
    sched::unshare(sched::CloneFlags::CLONE_NEWNET).map_err(|error| Error::System {
        context: "could not create a new network namespace".into(),
        error,
    })?;

    // Step 2.5: Bind-mount it to a persistent path. We can use /proc/self/ns/net because we're currently in the new namespace
    if let Err(error) = bind_mount("/proc/self/ns/net", &namespace_path) {
        // If the bind mount failed, we should clean up by removing the namespace file we created
        if let Err(err) = fs::remove_file(&namespace_path) {
            // TODO: log instead
            eprintln!(
                "could not clean up namespace file {} on failed creation: {:?}",
                namespace_path.display(),
                err
            );
        }

        return Err(error);
    }

    Ok(namespace_path)
}

/// Delete a network namespace.
pub fn delete(name: &str) -> Result<(), Error> {
    let path = persistent_namespace_path(name);
    // This will fail with EINVAL if the mount point has already been unbound
    let _ = umount2(&path, MntFlags::MNT_DETACH);
    fs::remove_file(&path).map_err(|error| Error::Io {
        context: format!("could not remove namespace file {}", path.display()),
        error,
    })
}

/// Prepare the root runtime directory for persistent network namespaces.
///
/// It's expected that network namespace mounts propagate between mount namespaces. This allows network namespaces to be freed sooner, since
/// unmounting the network namespace in one mount namespace will likely unmount it in all other mount namespaces.
///
/// To do this, we remount [`NETNS_RUNTIME_DIRECTORY`] with [`MsFlags::MS_SHARED`] and [`MsFlags::MS_REC`]. If it is not already a mount point, we make it one by
/// mounting it over itself with [`MsFlags::MS_BIND`] and [`MsFlags::MS_REC`].
fn prepare_runtime_directory() -> Result<(), Error> {
    // Adapted from create_netns_dir and netns_add in ipnetns.c from the iproute2 source code

    // Step 1: ensure that the runtime directory exists
    match DirBuilder::new()
        .mode(
            (Mode::S_IRWXU | Mode::S_IRGRP | Mode::S_IXGRP | Mode::S_IROTH | Mode::S_IXOTH).bits(),
        )
        .create(NETNS_RUNTIME_DIRECTORY)
    {
        Ok(()) => (),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => {
            return Err(Error::Io {
                context: format!("could not create {}", NETNS_RUNTIME_DIRECTORY),
                error,
            })
        }
    };

    // Step 2: lock the directory. This is in order to be a good citizen; if multiple processes try to set up the mountpoint at the same time, they can
    // cause it to be recursively created multiple times, locking up the system.
    let lock_file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY)
        .open(NETNS_RUNTIME_DIRECTORY)
        .map_err(|error| Error::Io {
            context: format!("could not open {}", NETNS_RUNTIME_DIRECTORY),
            error,
        })?;
    let _lock = FileLock::new(&lock_file).map_err(|error| Error::System {
        context: format!("could not lock {}", NETNS_RUNTIME_DIRECTORY),
        error,
    })?;

    // Step 3: Make the mountpoint shared, with recursive propagation
    fn set_propagation() -> Result<(), Error> {
        mount(
            NONE,
            NETNS_RUNTIME_DIRECTORY,
            NONE,
            MsFlags::MS_SHARED | MsFlags::MS_REC,
            NONE,
        )
        .map_err(|error| Error::System {
            context: format!(
                "could not set mount propagation on {}",
                NETNS_RUNTIME_DIRECTORY
            ),
            error,
        })
    }

    match set_propagation() {
        Err(Error::System {
            error: nix::Error::Sys(Errno::EINVAL),
            ..
        }) => {
            // If set_propagation failed with EINVAL, assume we need to upgrade to a mountpoint
            bind_mount_flags(
                NETNS_RUNTIME_DIRECTORY,
                NETNS_RUNTIME_DIRECTORY,
                MsFlags::MS_REC,
            )?;
            set_propagation()?;
        }
        Err(err) => return Err(err),
        Ok(()) => (),
    };

    Ok(())
}

/// Gets the path that a persistent network namespace should be bound to.
fn persistent_namespace_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(NETNS_RUNTIME_DIRECTORY);
    path.push(name);
    path
}

impl NamespaceGuard {
    /// Create a new [`NamespaceGuard`] that will restore the current network namespace of the process. This allows temporarily switching to another network
    /// namespace with [`sched::unshare`].
    fn from_current() -> Result<NamespaceGuard, Error> {
        let saved_namespace = OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_CLOEXEC)
            .open("/proc/self/ns/net")
            .map_err(|error| Error::Io {
                context: "could not open current network namespace".into(),
                error,
            })?;
        Ok(NamespaceGuard(saved_namespace))
    }
}

impl Drop for NamespaceGuard {
    fn drop(&mut self) {
        sched::setns(self.0.as_raw_fd(), sched::CloneFlags::CLONE_NEWNET)
            .expect("could not restore network namespace!")
    }
}
