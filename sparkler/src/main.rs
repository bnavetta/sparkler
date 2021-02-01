use std::error::Error as _;
use std::path::Path;

use nix::unistd::{Uid, Gid};
use tokio::task::spawn_blocking;

mod error;
mod firecracker;
mod network;
mod util;

use error::Error;
use firecracker::api::*;
use firecracker::jailer::{self, ConfigBuilder};

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

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let network_namespace = spawn_blocking(|| {
        network::namespace::create_network_namespace("test")
    }).await??;

    let jailer_config = ConfigBuilder::default()
        .jailer_binary(Path::new("/usr/bin/jailer"))
        .firecracker_binary(Path::new("/usr/bin/firecracker"))
        .chroot_base(Path::new("/srv/jailer"))
        .user(Uid::current())
        .group(Gid::current())
        .id("myvm")
        .network_namespace(network_namespace.as_path())
        .build()?;

    let image_path = jailer_config.chroot_path().join("image");
    std::fs::create_dir_all(&image_path)?;
    util::bind_mount("image", &image_path)?;

    println!("Starting Firecracker");
    let mut jail_proc = jailer::spawn(&jailer_config)?;

    let socket_path = jailer_config.chroot_path().join("run").join("firecracker.sock");

    println!("Waiting for Firecracker to start ({})...", socket_path.display());
    while !socket_path.exists() {
        tokio::task::yield_now().await;
    }

    println!("Firecracker up!");

    let client = Client::new(socket_path);

    client.set_boot_source(&BootSource {
        kernel_image_path: "image/hello-vmlinux.bin".into(),
        initrd_path: None,
        boot_args: Some("console=ttyS0 reboot=k panic=1 pci=off".into())
    }).await?;

    client.set_drive(&Drive {
        drive_id: "rootfs".into(),
        is_read_only: false,
        is_root_device: true,
        path_on_host: "image/hello-rootfs.ext4".into(),
        partuuid: None,
        rate_limiter: None,
    }).await?;

    client.action(ActionType::InstanceStart).await?;

    let info = client.instance_info().await?;
    println!("Instance info: {:?}", info);

    spawn_blocking(move || jail_proc.wait()).await??;

    Ok(())
}


async fn start_vm() -> Result<(), Error> {
    let client = Client::new("test.sock");

    println!("Starting VM...");

    client.set_boot_source(&BootSource {
        kernel_image_path: "hello-vmlinux.bin".into(),
        initrd_path: None,
        boot_args: Some("console=ttyS0 reboot=k panic=1 pci=off".into()),
    }).await?;

    client.set_drive(&Drive {
        drive_id: "rootfs".into(),
        is_read_only: false,
        is_root_device: true,
        path_on_host: "hello-rootfs.ext4".into(),
        partuuid: None,
        rate_limiter: None,
    }).await?;

    client.action(ActionType::InstanceStart).await?;

    let info = client.instance_info().await?;
    println!("Instance info: {:?}", info);

    Ok(())
}
