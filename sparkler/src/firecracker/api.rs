//! Client for the Firecracker HTTP API

// TODO: client-side logging/tracing
// TODO: Firecracker metrics, logger, mmds, maybe snapshots, maybe vsock

use std::path::PathBuf;

use http::StatusCode;
use hyper::body::{Body, Buf};
use hyperlocal::{UnixClientExt, Uri, UnixConnector};
use thiserror::Error;

pub use self::model::{BootSource, InstanceInfo, Drive, RateLimiter, TokenBucket, ActionType};
use self::model::InstanceActionInfo;

pub struct Client {
    socket_path: PathBuf,
    inner: hyper::Client<UnixConnector, Body>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP transport error")]
    Transport(#[from] hyper::Error),

    #[error("invalid JSON")]
    InvalidJson(#[from] serde_json::Error),

    #[error("unexpected HTTP response: {0}")]
    UnexpectedResponse(String),

    #[error("client error: {fault_message}")]
    Client {
        fault_message: String,
    },

    #[error("server error: {fault_message}")]
    Server {
        fault_message: String,
    }
}

impl Client {
    pub fn new<P: Into<PathBuf>>(socket_path: P) -> Client {
        let inner = hyper::Client::unix();
        Client {
            socket_path: socket_path.into(),
            inner
        }
    }

    /// Returns general information about an instance.
    pub async fn instance_info(&self) -> Result<InstanceInfo, Error> {
        let request = self.builder_for("/")
            .method("GET")
            .body(Body::default())
            .expect("malformed request");
        let response = self.inner.request(request).await?;
        if response.status() == StatusCode::OK {
            deserialize_json(response).await
        } else {
            Err(deserialize_error(response).await)
        }
    }


    /// Creates new boot source if one does not already exist, otherwise updates it.
    /// Will fail if update is not possible. Pre-boot only.
    pub async fn set_boot_source(&self, source: &BootSource) -> Result<(), Error> {
        let request = self.builder_for("/boot-source")
            .method("PUT")
            .body(serialize_json(source))
            .expect("malformed request");
        let response = self.inner.request(request).await?;
        if response.status() == StatusCode::NO_CONTENT {
            Ok(())
        } else {
            Err(deserialize_error(response).await)
        }
    }

    /// Creates new drive with ID specified by the specified drive ID.
    /// If a drive with the specified ID already exists, updates its state based on new input.
    /// Will fail if update is not possible.
    pub async fn set_drive(&self, drive: &Drive) -> Result<(), Error> {
        // TODO: can the drive ID in the URL ever be different from the drive ID in the body?
        let request = self.builder_for(&format!("/drives/{}", drive.drive_id))
            .method("PUT")
            .body(serialize_json(drive))
            .expect("malformed request");
        let response = self.inner.request(request).await?;
        if response.status() == StatusCode::NO_CONTENT {
            Ok(())
        } else {
            Err(deserialize_error(response).await)
        }
    }

    /// Creates a synchronous (to the VMM) action.
    pub async fn action(&self, action: ActionType) -> Result<(), Error> {
        let request = self.builder_for("/actions")
            .method("PUT")
            .body(serialize_json(&InstanceActionInfo { action_type: action }))
            .expect("malformed request");
        let response = self.inner.request(request).await?;
        if response.status() == StatusCode::NO_CONTENT {
            Ok(())
        } else {
            Err(deserialize_error(response).await)
        }
    }

    fn builder_for(&self, path: &str) -> http::request::Builder {
        http::Request::builder()
            .uri(hyper::Uri::from(Uri::new(&self.socket_path, path)))
            .header(http::header::ACCEPT, "application/json")
            .header(http::header::CONTENT_TYPE, "application/json")
    }
}

/// Serialize a value to a JSON body
fn serialize_json<S: serde::Serialize>(body: &S) -> Body {
    serde_json::to_vec(body)
        .expect("malformed body")
        .into()
}

/// Deserializes the HTTP response body as JSON
async fn deserialize_json<D: serde::de::DeserializeOwned>(response: hyper::Response<Body>) -> Result<D, Error> {
    let body = hyper::body::aggregate(response).await?;
    Ok(serde_json::from_reader(body.reader())?)
}

async fn deserialize_error(response: hyper::Response<Body>) -> Error {
    let status = response.status();
    let error: model::Error = match deserialize_json(response).await {
        Ok(err) => err,
        Err(err) => return err
    };
    if status.is_client_error() {
        Error::Client { fault_message: error.fault_message }
    } else if status.is_server_error() {
        Error::Server { fault_message: error.fault_message }
    } else {
        Error::UnexpectedResponse(format!("Got {} from server, expected an error", status))
    }
}


/// Firecracker API model types, from the [spec](https://github.com/firecracker-microvm/firecracker/blob/master/src/api_server/swagger/firecracker.yaml).
mod model {
    use std::path::PathBuf;

    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
    pub struct Error {
        /// A description of the error condition
        pub fault_message: String,
    }

    /// Describes MicroVM instance information.
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
    pub struct InstanceInfo {
        /// Application name.
        pub app_name: String,
        /// MicroVM / instance ID.
        pub id: String,
        /// The current detailed state (Not started, Running, Paused) of the Firecracker instance.
        /// This value is read-only for the control-plane.
        pub state: InstanceState,
        /// MicroVM hypervisor build version.
        pub vmm_version: String,
    }

    /// Instance state, as part of the [`InstanceInfo`] response.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Deserialize)]
    pub enum InstanceState {
        #[serde(rename = "Not started")]
        NotStarted,
        Running,
        Paused,
    }

    /// Variant wrapper containing the real action.
    /// Used for the `/actions` endpoint.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
    pub struct InstanceActionInfo {
        pub action_type: ActionType,
    }

    /// Enumeration indicating what type of action is contained in the payload
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
    pub enum ActionType {
        FlushMetrics,
        InstanceStart,
        SendCtrlAltDel,
    }

    /// Boot source descriptor.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
    pub struct BootSource {
        /// Kernel boot arguments
        #[serde(skip_serializing_if = "Option::is_none")]
        pub boot_args: Option<String>,
        /// Host level path to the initrd image used to boot the guest
        #[serde(skip_serializing_if = "Option::is_none")]
        pub initrd_path: Option<PathBuf>,
        /// Host level path to the kernel image used to boot the guest
        pub kernel_image_path: PathBuf,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
    pub struct Drive {
        pub drive_id: String,
        pub is_read_only: bool,
        pub is_root_device: bool,
        /// Represents the unique id of the boot partition of this device. It is
        /// optional and it will be taken into account only if the [`is_root_device`]
        /// field is true.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub partuuid: Option<String>,
        pub path_on_host: PathBuf,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub rate_limiter: Option<RateLimiter>,
    }

    /// Defines an IO rate limiter with independent bytes/s and ops/s limits.
    /// Limits are defined by configuring each of the _bandwidth_ and _ops_ token buckets.
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Default)]
    pub struct RateLimiter {
        /// Token bucket with bytes as tokens
        #[serde(skip_serializing_if = "Option::is_none")]
        pub bandwidth: Option<TokenBucket>,
        /// Token bucket with operations as tokens
        #[serde(skip_serializing_if = "Option::is_none")]
        pub ops: Option<TokenBucket>,
    }

    /// Defines a token bucket with a maximum capacity (size), an initial burst size
    /// (one_time_burst) and an interval for refilling purposes (refill_time).
    /// The refill-rate is derived from size and refill_time, and it is the constant
    /// rate at which the tokens replenish. The refill process only starts happening after
    /// the initial burst budget is consumed.
    /// Consumption from the token bucket is unbounded in speed which allows for bursts
    /// bound in size by the amount of tokens available.
    /// Once the token bucket is empty, consumption speed is bound by the refill_rate.
    #[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
    pub struct TokenBucket {
        /// The initial size of a token bucket.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub one_time_burst: Option<u64>,

        /// The amount of milliseconds it takes for the bucket to refill.
        pub refill_time: u64,

        /// The total number of tokens this bucket can hold.
        pub size: u64,
    }   
}