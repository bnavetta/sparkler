use thiserror::Error;

use crate::firecracker::api;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Firecracker API error")]
    Api(#[from] api::Error),

    #[error("i/o error: {context}")]
    Io {
        context: String,
        #[source]
        error: std::io::Error,
    },

    #[error("system error: {context}")]
    System {
        context: String,
        #[source]
        error: nix::Error,
    },

    #[error("jailer error")]
    Jailer(unshare::Error),
}