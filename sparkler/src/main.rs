use std::{env, fs, time::Duration};
use std::path::PathBuf;

use nix::unistd::{Gid, Uid};
use tokio::task::spawn_blocking;
use tokio::time;
use tracing::{error, info};

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
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let state = match spawn_blocking(setup_vm).await.unwrap() {
        Ok(state) => state,
        Err(error) => die(&error),
    };

    if let Err(error) = run(&state).await {
        error!("run() failed: {}", error);
        if let Err(cleanup_err) = spawn_blocking(|| cleanup_vm(state)).await.unwrap() {
            error!("Cleanup failed: {}", cleanup_err);
        }
        die(&error);
    }

    if let Err(error) = spawn_blocking(|| cleanup_vm(state)).await.unwrap() {
        die(&error);
    }
}

fn die(error: &dyn std::error::Error) -> ! {
    let cause = match error.source() {
        Some(cause) => format!("\ncause: {}", cause),
        None => "".into(),
    };
    error!("Error: {}{}", error, cause);
    std::process::exit(1);
}

#[derive(Debug)]
struct VmState {
    process: unshare::Child,
    chroot_path: PathBuf,
}

#[tracing::instrument]
fn setup_vm() -> Result<VmState, Error> {
    let network_namespace = network::namespace::create(NETWORK_NAMESPACE)?;

    let jailer_config = ConfigBuilder::default()
        .user(Uid::current())
        .group(Gid::current())
        .id("testvm")
        .network_namespace(network_namespace.as_path())
        .build()
        .unwrap();

    let image_path = jailer_config.chroot_path().join("image");
    fs::create_dir_all(&image_path).map_err(|error| Error::Io {
        context: format!("could not create image directory {}", image_path.display()),
        error,
    })?;

    util::bind_mount("./image", &image_path)?;

    info!("Starting Firecracker");
    let process = jailer::spawn(&jailer_config)?;

    Ok(VmState {
        process,
        chroot_path: jailer_config.chroot_path(),
    })
}

#[tracing::instrument]
fn cleanup_vm(mut state: VmState) -> Result<(), Error> {
    use nix::mount::{umount2, MntFlags};

    info!("Cleaning up VMM");

    state.process.signal(unshare::Signal::SIGKILL)
        .map_err(|error| Error::Io {
            context: format!("could not kill jailer process {}", state.process.pid()),
            error
        })?;
    let exit_status = state.process.wait().map_err(|error| Error::Io {
        context: format!("waiting for jailer process {} failed", state.process.pid()),
        error,
    })?;
    info!("Jailer exited with status {}", exit_status);

    network::namespace::delete(NETWORK_NAMESPACE)?;

    let images_dir = state.chroot_path.join("image");
    umount2(&images_dir, MntFlags::MNT_DETACH).map_err(|error| Error::System {
        context: format!("could not unmount image directory {}", images_dir.display()),
        error,
    })?;

    // The chroot is in the "root" subdirectory of the VM's state path.
    let state_root = state.chroot_path.parent().unwrap();
    fs::remove_dir_all(&state_root).map_err(|error| Error::Io {
        context: format!("could not remove VM  state in {}", state_root.display()),
        error,
    })?;

    Ok(())
}

async fn run(state: &VmState) -> Result<(), Error> {
    let socket_path = state.chroot_path.join("run").join("firecracker.socket");

    let mut exists = false;
    for _ in 0u8..10 {
        if socket_path.exists() {
            exists = true;
            break;
        }

        time::sleep(Duration::from_secs(1)).await;
    }

    if !exists {
        return Err(Error::Api(firecracker::api::Error::Server {
            fault_message: "timed out waiting for socket to exist".into()
        }))
    }

    info!(
        "Attempting to communicate with VMM at {}",
        socket_path.display()
    );

    let client = Client::new(socket_path);

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

    let info = client.instance_info().await?;
    info!("Instance info: {:?}", info);

    Ok(())
}
