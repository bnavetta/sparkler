use std::{error::Error as _, path::PathBuf};
use std::path::Path;
use std::fs;

use nix::unistd::{Uid, Gid};
use tokio::task::spawn_blocking;

mod error;
mod firecracker;
mod network;
mod util;

use error::Error;
use firecracker::api::*;
use firecracker::jailer::{self, ConfigBuilder};

const NETWORK_NAMESPACE: &str = "test";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        let cause = match error.source() {
            Some(cause) => format!("\ncause: {}", cause),
            None => "".into()
        };
        panic!("Error: {}{}", error, cause);
    }
}

struct VmState {
    process: unshare::Child,
    chroot_path: PathBuf,
}

fn setup_vm() -> Result<VmState, Error> {
    let network_namespace = network::namespace::create(NETWORK_NAMESPACE)?;

    let jailer_config = ConfigBuilder::default()
        .user(Uid::current())
        .group(Gid::current())
        .id("testvm")
        .network_namespace(network_namespace.as_path())
        .build().unwrap();

    let image_path = jailer_config.chroot_path().join("image");
    fs::create_dir_all(&image_path).map_err(|error| Error::Io {
        context: format!("could not create image directory {}", image_path.display()),
        error
    })?;

    util::bind_mount("./image", &image_path)?;

    println!("Starting Firecracker");
    let process = jailer::spawn(&jailer_config)?;

    Ok(VmState {
        process,
        chroot_path: jailer_config.chroot_path(),
    })
}

fn cleanup_vm(state: VmState) -> Result<(), Error> {
    use nix::mount::{umount2, MntFlags};

    network::namespace::delete(NETWORK_NAMESPACE)?;

    let images_dir = state.chroot_path.join("image");
    umount2(&images_dir, MntFlags::MNT_DETACH)
        .map_err(|error| Error::System {
            context: format!("could not unmount image directory {}", images_dir.display()),
            error,
        })?;

    // The chroot is in the "root" subdirectory of the VM's state path.
    let state_root = state.chroot_path.parent().unwrap();
    fs::remove_dir_all(&state_root)
        .map_err(|error| Error::Io {
            context: format!("could not remove VM  state in {}", state_root.display()),
            error
        })?;

    Ok(())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let state = spawn_blocking(setup_vm).await??;

    // let socket_path = state.chroot_path.join("run").join("firecracker.sock");

    // let client = Client::new(socket_path);

    // client.set_boot_source(&BootSource {
    //     kernel_image_path: "image/hello-vmlinux.bin".into(),
    //     initrd_path: None,
    //     boot_args: Some("console=ttyS0 reboot=k panic=1 pci=off".into())
    // }).await?;

    // client.set_drive(&Drive {
    //     drive_id: "rootfs".into(),
    //     is_read_only: false,
    //     is_root_device: true,
    //     path_on_host: "image/hello-rootfs.ext4".into(),
    //     partuuid: None,
    //     rate_limiter: None,
    // }).await?;

    // client.action(ActionType::InstanceStart).await?;

    // let info = client.instance_info().await?;
    // println!("Instance info: {:?}", info);


    spawn_blocking(|| cleanup_vm(state)).await??;

    Ok(())
}
