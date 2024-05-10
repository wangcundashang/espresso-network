use std::sync::Arc;

use async_broadcast::{SendError, Sender};
#[cfg(async_executor_impl = "async-std")]
use async_std::task::{spawn_blocking, JoinHandle};
use hotshot_types::{
    data::VidDisperse,
    traits::{election::Membership, node_implementation::NodeType},
    vid::{vid_scheme, VidPrecomputeData},
};
use jf_vid::{precomputable::Precomputable, VidScheme};
#[cfg(async_executor_impl = "tokio")]
use tokio::task::{spawn_blocking, JoinHandle};

/// Cancel a task
pub async fn cancel_task<T>(task: JoinHandle<T>) {
    #[cfg(async_executor_impl = "async-std")]
    task.cancel().await;
    #[cfg(async_executor_impl = "tokio")]
    task.abort();
}

/// Helper function to send events and log errors
pub async fn broadcast_event<E: Clone + std::fmt::Debug>(event: E, sender: &Sender<E>) {
    match sender.broadcast_direct(event).await {
        Ok(None) => (),
        Ok(Some(overflowed)) => {
            tracing::error!(
                "Event sender queue overflow, Oldest event removed form queue: {:?}",
                overflowed
            );
        }
        Err(SendError(e)) => {
            tracing::warn!(
                "Event: {:?}\n Sending failed, event stream probably shutdown",
                e
            );
        }
    }
}

/// Calculate the vid disperse information from the payload given a view and membership,
/// optionally using precompute data from builder
///
/// # Panics
/// Panics if the VID calculation fails, this should not happen.
#[allow(clippy::panic)]
pub async fn calculate_vid_disperse<TYPES: NodeType>(
    txns: Arc<[u8]>,
    membership: &Arc<TYPES::Membership>,
    view: TYPES::Time,
    precompute_data: Option<VidPrecomputeData>,
) -> VidDisperse<TYPES> {
    let num_nodes = membership.total_nodes();

    let vid_disperse = spawn_blocking(move || {
        precompute_data
            .map_or_else(
                || vid_scheme(num_nodes).disperse(Arc::clone(&txns)),
                |data| vid_scheme(num_nodes).disperse_precompute(Arc::clone(&txns), &data)
            )
            .unwrap_or_else(|err| panic!("VID disperse failure:(num_storage nodes,payload_byte_len)=({num_nodes},{}) error: {err}", txns.len()))
    }).await;
    #[cfg(async_executor_impl = "tokio")]
    // Tokio's JoinHandle's `Output` is `Result<T, JoinError>`, while in async-std it's just `T`
    // Unwrap here will just propagate any panic from the spawned task, it's not a new place we can panic.
    let vid_disperse = vid_disperse.unwrap();

    VidDisperse::from_membership(view, vid_disperse, membership.as_ref())
}

/// Utilities to print anyhow logs.
pub trait AnyhowTracing {
    /// Print logs as debug
    fn err_as_debug(self);
}

impl<T> AnyhowTracing for anyhow::Result<T> {
    fn err_as_debug(self) {
        let _ = self.inspect_err(|e| tracing::debug!("{}", format!("{:?}", e)));
    }
}
