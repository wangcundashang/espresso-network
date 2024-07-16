use async_broadcast::{broadcast, InactiveReceiver, Sender as BroadcastSender};
use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::stream::{BoxStream, Stream, StreamExt};
use hotshot_types::{
    data::{DaProposal, QuorumProposal},
    error::HotShotError,
    event::{error_adaptor, Event, EventType},
    message::Proposal,
    traits::node_implementation::NodeType,
    PeerConfig,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tide_disco::method::ReadState;
const RETAINED_EVENTS_COUNT: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound(deserialize = "Types: NodeType"))]
pub enum BuilderEventType<Types: NodeType> {
    // Information required by the builder to create a membership to get view leader
    StartupInfo {
        known_node_with_stake: Vec<PeerConfig<Types::SignatureKey>>,
        non_staked_node_count: usize,
    },
    /// Hotshot error
    HotshotError {
        /// The underlying error
        #[serde(with = "error_adaptor")]
        error: Arc<HotShotError<Types>>,
    },
    /// Hotshot public mempool transactions
    HotshotTransactions {
        /// The list of hotshot transactions
        transactions: Vec<Types::Transaction>,
    },
    // Decide event with the chain of decided leaves
    HotshotDecide {
        /// The chain of decided leaves with its corresponding state and VID info.
        latest_decide_view_num: Types::Time,
        /// Optional information of the number of transactions in the block
        block_size: Option<u64>,
    },
    /// DA proposal was received from the network
    HotshotDaProposal {
        /// Contents of the proposal
        proposal: Proposal<Types, DaProposal<Types>>,
        /// Public key of the leader submitting the proposal
        sender: Types::SignatureKey,
    },
    /// Quorum proposal was received from the network
    HotshotQuorumProposal {
        /// Contents of the proposal
        proposal: Proposal<Types, QuorumProposal<Types>>,
        /// Public key of the leader submitting the proposal
        sender: Types::SignatureKey,
    },
    Unknown,
}

#[async_trait]
pub trait EventsSource<Types>
where
    Types: NodeType,
{
    type EventStream: Stream<Item = Arc<Event<Types>>> + Unpin + Send + 'static;
    async fn get_event_stream(&self) -> Self::EventStream;

    async fn subscribe_events(&self) -> BoxStream<'static, Arc<Event<Types>>> {
        self.get_event_stream().await.boxed()
    }
}

#[async_trait]
pub trait EventConsumer<Types>
where
    Types: NodeType,
{
    async fn handle_event(&mut self, event: Event<Types>);
}

#[derive(Debug)]
pub struct EventsStreamer<Types: NodeType> {
    // required for api subscription
    inactive_to_subscribe_clone_recv: InactiveReceiver<Arc<Event<Types>>>,
    subscriber_send_channel: BroadcastSender<Arc<Event<Types>>>,

    // required for sending startup info
    known_nodes_with_stake: Vec<PeerConfig<Types::SignatureKey>>,
    non_staked_node_count: usize,
}

impl<Types: NodeType> EventsStreamer<Types> {
    pub fn known_node_with_stake(&self) -> Vec<PeerConfig<Types::SignatureKey>> {
        self.known_nodes_with_stake.clone()
    }

    pub fn non_staked_node_count(&self) -> usize {
        self.non_staked_node_count
    }
}

#[async_trait]
impl<Types: NodeType> EventConsumer<Types> for EventsStreamer<Types> {
    async fn handle_event(&mut self, event: Event<Types>) {
        let filter = match event {
            Event {
                event: EventType::DaProposal { .. },
                ..
            } => true,
            Event {
                event: EventType::QuorumProposal { .. },
                ..
            } => true,
            Event {
                event: EventType::Transactions { .. },
                ..
            } => true,
            Event {
                event: EventType::Decide { .. },
                ..
            } => true,
            Event { .. } => false,
        };
        if filter {
            let _status = self.subscriber_send_channel.broadcast(event.into()).await;
        }
    }
}

#[async_trait]
impl<Types: NodeType> EventsSource<Types> for EventsStreamer<Types> {
    type EventStream = BoxStream<'static, Arc<Event<Types>>>;

    async fn get_event_stream(&self) -> Self::EventStream {
        self.inactive_to_subscribe_clone_recv
            .activate_cloned()
            .boxed()
    }
}
impl<Types: NodeType> EventsStreamer<Types> {
    pub fn new(
        known_nodes_with_stake: Vec<PeerConfig<Types::SignatureKey>>,
        non_staked_node_count: usize,
    ) -> Self {
        let (mut subscriber_send_channel, to_subscribe_clone_recv) =
            broadcast::<Arc<Event<Types>>>(RETAINED_EVENTS_COUNT);
        // set the overflow to true to drop older messages from the channel
        subscriber_send_channel.set_overflow(true);
        // set the await active to false to not block the sender
        subscriber_send_channel.set_await_active(false);
        let inactive_to_subscribe_clone_recv = to_subscribe_clone_recv.deactivate();
        EventsStreamer {
            subscriber_send_channel,
            inactive_to_subscribe_clone_recv,
            known_nodes_with_stake,
            non_staked_node_count,
        }
    }
}

#[async_trait]
impl<Types: NodeType> ReadState for EventsStreamer<Types> {
    type State = Self;

    async fn read<T>(
        &self,
        op: impl Send + for<'a> FnOnce(&'a Self::State) -> BoxFuture<'a, T> + 'async_trait,
    ) -> T {
        op(self).await
    }
}
