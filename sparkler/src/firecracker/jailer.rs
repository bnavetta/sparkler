use std::{collections::HashMap, ffi::OsString, path::{Path, PathBuf}};

use nix::unistd::{Gid, Uid};
use unshare::{Command, Child, Namespace};

use crate::Error;

// TODO: systemd-like dynamic users?

const DEFAULT_JAILER: &str = "/usr/bin/jailer";
const DEFAULT_FIRECRACKER: &str = "/usr/bin/firecracker";
const DEFAULT_CHROOT_BASE: &str = "/srv/jailer";

/// Firecracker jail configuration.
///
/// This takes references to most settings, as they will generally be reused across microVMs.
#[derive(derive_builder::Builder)]
pub struct Config<'a> {
    /// Path to the `jailer` executable
    #[builder(default = "{ Path::new(DEFAULT_JAILER) }")]
    jailer_binary: &'a Path,

    /// Path to the `firecracker` executable
    #[builder(default = "{ Path::new(DEFAULT_FIRECRACKER) }")]
    firecracker_binary: &'a Path,

    /// Unique microVM jail ID
    id: &'a str,

    /// User to switch to when running Firecracker
    user: Uid,

    /// Group to switch to when running Firecracker
    group: Gid,

    /// Base directory to create chroot jails under.
    #[builder(default = "{ Path::new(DEFAULT_CHROOT_BASE) }")]
    chroot_base: &'a Path,

    /// Network namespace to join before running Firecracker
    #[builder(setter(strip_option))]
    network_namespace: Option<&'a Path>,

    /// cgroup settings to apply to the Firecracker process. Keys are cgroup file names (like `cpuset.cpus`) and values are the file contents
    /// (such as `0` to bind to CPU 0).
    #[builder(default = "{ HashMap::new() }")]
    cgroup: HashMap<String, String>,

    /// Command-line arguments for Firecracker. Note that the jailer passes some additional arguments such as `--id`.
    #[builder(default = "{ Vec::new() }")]
    firecracker_args: Vec<OsString>,
}

impl<'a> Config<'a> {
    /// Creates a new jailer configuration with the given microVM ID, user ID, and group ID. This uses the default paths for the jailer binary,
    /// Firecracker binary, and chroot base.
    fn new(id: &'a str, user: Uid, group: Gid) -> Config<'a> {
        Config {
            jailer_binary: Path::new(DEFAULT_JAILER),
            firecracker_binary: Path::new(DEFAULT_FIRECRACKER),
            id,
            user,
            group,
            chroot_base: Path::new(DEFAULT_CHROOT_BASE),
            network_namespace: None,
            cgroup: HashMap::new(),
            firecracker_args: Vec::new(),
        }
    }

    /// Directory that the jailer will chroot into before running Firecracker.
    ///
    /// This takes the form `$chroot_base/$(basename $firecracker_binary)/$id`.
    pub fn chroot_path(&self) -> PathBuf {
        let mut path = self.chroot_base.to_path_buf();
        path.push(self.firecracker_binary
            .file_name()
            .expect("no file name for Firecracker binary"));
        path.push(self.id);
        path.push("root");
        path
    }
}

fn build_command(config: &Config<'_>) -> Command {
    // Use `unshare` for starting the jailer, since it handles the nuances of safely `clone()`ing from Rust. The alternative would be doing the
    // clone(CLONE_NEWPID) -> exec dance ourselves, while making sure not to accidentally deadlock or break things.
    let mut command = Command::new(config.jailer_binary);
    
    command.arg("--id").arg(config.id);
    command.arg("--exec-file").arg(config.firecracker_binary);
    command.arg("--uid").arg(config.user.to_string());
    command.arg("--gid").arg(config.group.to_string());
    command.arg("--chroot-base-dir").arg(config.chroot_base);

    for (file, value) in config.cgroup.iter() {
        command.arg("--cgroup").arg(format!("{}={}", file, value));
    }
    
    if let Some(netns) = config.network_namespace {
        command.arg("--netns").arg(netns);
    }

    if !config.firecracker_args.is_empty() {
        command.arg("--");
        command.args(&config.firecracker_args);
    }

    command.unshare(&[Namespace::Pid]);

    command
}

pub fn spawn(config: &Config<'_>) -> Result<Child, Error> {
    build_command(config)
        .spawn()
        .map_err(Error::Jailer)
}
