use crate::infra::{load_config_from_file, OrchestratorArgs};

use async_compatibility_layer::logging::{setup_backtrace, setup_logging};
use async_lock::RwLock;
use async_trait::async_trait;
use futures::StreamExt;
use hotshot::{
    traits::{
        implementations::{
            Libp2pCommChannel, Libp2pNetwork, MemoryStorage, WebCommChannel, WebServerNetwork,
        },
        NodeImplementation,
    },
    types::{SignatureKey, SystemContextHandle},
    HotShotType, SystemContext,
};
use hotshot_orchestrator::{
    self,
    client::{OrchestratorClient, ValidatorArgs},
    config::{NetworkConfig, WebServerConfig},
};
use hotshot_task::task::FilterEvent;
use hotshot_types::HotShotConfig;
use hotshot_types::{
    certificate::ViewSyncCertificate,
    data::{QuorumProposal, SequencingLeaf, TestableLeaf},
    event::{Event, EventType},
    message::{Message, SequencingMessage},
    traits::{
        election::{
            CommitteeExchange, ConsensusExchange, Membership, QuorumExchange, ViewSyncExchange,
        },
        metrics::NoMetrics,
        network::CommunicationChannel,
        node_implementation::{
            CommitteeEx, ExchangesType, NodeType, QuorumEx, SequencingExchanges,
        },
        state::{ConsensusTime, TestableBlock, TestableState},
    },
};
use libp2p_identity::{
    ed25519::{self, SecretKey},
    Keypair,
};
use libp2p_networking::{
    network::{MeshParams, NetworkNodeConfigBuilder, NetworkNodeType},
    reexport::Multiaddr,
};
use std::{collections::BTreeSet, sync::Arc};
use std::{num::NonZeroUsize, str::FromStr};
// use libp2p::{
//     identity::{
//         ed25519::{Keypair as EdKeypair, SecretKey},
//         Keypair,
//     },
//     multiaddr::{self, Protocol},
//     Multiaddr,
// };
use libp2p_identity::PeerId;
// use libp2p_networking::network::{MeshParams, NetworkNodeConfigBuilder, NetworkNodeType};
use std::{
    //collections::{BTreeSet, VecDeque},
    collections::VecDeque,
    //fs,
    mem,
    net::IpAddr,
    //num::NonZeroUsize,
    //str::FromStr,
    //sync::Arc,
    //time::{Duration, Instant},
    time::Instant,
};
use std::{fmt::Debug, net::Ipv4Addr};
//use surf_disco::error::ClientError;
//use surf_disco::Client;
use tracing::{debug, error, info, warn};

/// Runs the orchestrator
pub async fn run_orchestrator_da<
    TYPES: NodeType,
    MEMBERSHIP: Membership<TYPES> + Debug,
    DANETWORK: CommunicationChannel<TYPES, Message<TYPES, NODE>, MEMBERSHIP> + Debug,
    QUORUMNETWORK: CommunicationChannel<TYPES, Message<TYPES, NODE>, MEMBERSHIP> + Debug,
    VIEWSYNCNETWORK: CommunicationChannel<TYPES, Message<TYPES, NODE>, MEMBERSHIP> + Debug,
    NODE: NodeImplementation<
        TYPES,
        Leaf = SequencingLeaf<TYPES>,
        Exchanges = SequencingExchanges<
            TYPES,
            Message<TYPES, NODE>,
            QuorumExchange<
                TYPES,
                SequencingLeaf<TYPES>,
                QuorumProposal<TYPES, SequencingLeaf<TYPES>>,
                MEMBERSHIP,
                QUORUMNETWORK,
                Message<TYPES, NODE>,
            >,
            CommitteeExchange<TYPES, MEMBERSHIP, DANETWORK, Message<TYPES, NODE>>,
            ViewSyncExchange<
                TYPES,
                ViewSyncCertificate<TYPES>,
                MEMBERSHIP,
                VIEWSYNCNETWORK,
                Message<TYPES, NODE>,
            >,
        >,
        Storage = MemoryStorage<TYPES, SequencingLeaf<TYPES>>,
        ConsensusMessage = SequencingMessage<TYPES, NODE>,
    >,
>(
    OrchestratorArgs {
        host,
        port,
        config_file,
    }: OrchestratorArgs,
) {
    error!("Starting orchestrator",);
    let run_config = load_config_from_file::<TYPES>(config_file);
    let _result = hotshot_orchestrator::run_orchestrator::<
        TYPES::SignatureKey,
        TYPES::ElectionConfigType,
    >(run_config, host, port)
    .await;
}

/// Defines the behavior of a "run" of the network with a given configuration
#[async_trait]
pub trait RunDA<
    TYPES: NodeType,
    MEMBERSHIP: Membership<TYPES> + Debug,
    DANETWORK: CommunicationChannel<TYPES, Message<TYPES, NODE>, MEMBERSHIP> + Debug,
    QUORUMNETWORK: CommunicationChannel<TYPES, Message<TYPES, NODE>, MEMBERSHIP> + Debug,
    VIEWSYNCNETWORK: CommunicationChannel<TYPES, Message<TYPES, NODE>, MEMBERSHIP> + Debug,
    NODE: NodeImplementation<
        TYPES,
        Leaf = SequencingLeaf<TYPES>,
        Exchanges = SequencingExchanges<
            TYPES,
            Message<TYPES, NODE>,
            QuorumExchange<
                TYPES,
                SequencingLeaf<TYPES>,
                QuorumProposal<TYPES, SequencingLeaf<TYPES>>,
                MEMBERSHIP,
                QUORUMNETWORK,
                Message<TYPES, NODE>,
            >,
            CommitteeExchange<TYPES, MEMBERSHIP, DANETWORK, Message<TYPES, NODE>>,
            ViewSyncExchange<
                TYPES,
                ViewSyncCertificate<TYPES>,
                MEMBERSHIP,
                VIEWSYNCNETWORK,
                Message<TYPES, NODE>,
            >,
        >,
        Storage = MemoryStorage<TYPES, SequencingLeaf<TYPES>>,
        ConsensusMessage = SequencingMessage<TYPES, NODE>,
    >,
> where
    <TYPES as NodeType>::StateType: TestableState,
    <TYPES as NodeType>::BlockType: TestableBlock,
    SequencingLeaf<TYPES>: TestableLeaf,
    Self: Sync,
    SystemContext<TYPES, NODE>: HotShotType<TYPES, NODE>,
{
    /// Initializes networking, returns self
    async fn initialize_networking(
        config: NetworkConfig<
            TYPES::SignatureKey,
            <TYPES::SignatureKey as SignatureKey>::StakeTableEntry,
            TYPES::ElectionConfigType,
        >,
    ) -> Self;

    /// Initializes the genesis state and HotShot instance; does not start HotShot consensus
    /// # Panics if it cannot generate a genesis block, fails to initialize HotShot, or cannot
    /// get the anchored view
    /// Note: sequencing leaf does not have state, so does not return state
    async fn initialize_state_and_hotshot(&self) -> SystemContextHandle<TYPES, NODE> {
        let genesis_block = TYPES::BlockType::genesis();
        let initializer =
            hotshot::HotShotInitializer::<TYPES, SequencingLeaf<TYPES>>::from_genesis(
                genesis_block,
            )
            .expect("Couldn't generate genesis block");

        let config = self.get_config();

        // Get KeyPair for certificate Aggregation
        let (pk, sk) =
            TYPES::SignatureKey::generated_from_seed_indexed(config.seed, config.node_index);
        let known_nodes_with_stake = config.config.known_nodes_with_stake.clone();
        let entry = pk.get_stake_table_entry(1u64);

        let da_network = self.get_da_network();
        let quorum_network = self.get_quorum_network();
        let view_sync_network = self.get_view_sync_network();

        // Since we do not currently pass the election config type in the NetworkConfig, this will always be the default election config
        let quorum_election_config = config.config.election_config.clone().unwrap_or_else(|| {
            <QuorumEx<TYPES,NODE> as ConsensusExchange<
                TYPES,
                Message<TYPES, NODE>,
            >>::Membership::default_election_config(config.config.total_nodes.get() as u64)
        });

        let committee_election_config = <CommitteeEx<TYPES, NODE> as ConsensusExchange<
            TYPES,
            Message<TYPES, NODE>,
        >>::Membership::default_election_config(
            config.config.da_committee_size.try_into().unwrap(),
        );

        let exchanges = NODE::Exchanges::create(
            known_nodes_with_stake.clone(),
            (quorum_election_config, committee_election_config),
            (
                quorum_network.clone(),
                da_network.clone(),
                view_sync_network.clone(),
            ),
            pk.clone(),
            entry.clone(),
            sk.clone(),
        );

        SystemContext::init(
            pk,
            sk,
            config.node_index,
            config.config,
            MemoryStorage::empty(),
            exchanges,
            initializer,
            NoMetrics::boxed(),
        )
        .await
        .expect("Could not init hotshot")
        .0
    }

    /// Starts HotShot consensus, returns when consensus has finished
    async fn run_hotshot(&self, mut context: SystemContextHandle<TYPES, NODE>) {
        let NetworkConfig {
            padding,
            rounds,
            transactions_per_round,
            node_index,
            config: HotShotConfig { total_nodes, .. },
            ..
        } = self.get_config();

        let size = mem::size_of::<TYPES::Transaction>();
        let adjusted_padding = if padding < size { 0 } else { padding - size };
        let mut txns: VecDeque<TYPES::Transaction> = VecDeque::new();

        let tx_to_gen = transactions_per_round * rounds * 3;
        {
            let mut txn_rng = rand::thread_rng();
            for _ in 0..tx_to_gen {
                let txn =
                    <<TYPES as NodeType>::StateType as TestableState>::create_random_transaction(
                        None,
                        &mut txn_rng,
                        padding as u64,
                    );
                txns.push_back(txn);
            }
        }
        debug!("Generated {} transactions", tx_to_gen);

        debug!("Adjusted padding size is {:?} bytes", adjusted_padding);
        let mut round = 0;
        let mut total_transactions = 0;

        let start = Instant::now();

        info!("Starting hotshot!");
        let (mut event_stream, _streamid) = context.get_event_stream(FilterEvent::default()).await;
        let mut anchor_view: TYPES::Time = <TYPES::Time as ConsensusTime>::genesis();
        let mut num_successful_commits = 0;

        let total_nodes_u64 = total_nodes.get() as u64;

        context.hotshot.start_consensus().await;

        loop {
            match event_stream.next().await {
                None => {
                    panic!("Error! Event stream completed before consensus ended.");
                }
                Some(Event { event, .. }) => {
                    match event {
                        EventType::Error { error } => {
                            error!("Error in consensus: {:?}", error);
                            // TODO what to do here
                        }
                        EventType::Decide {
                            leaf_chain,
                            qc: _,
                            block_size,
                        } => {
                            // this might be a obob
                            if let Some(leaf) = leaf_chain.get(0) {
                                info!("Decide event for leaf: {}", *leaf.view_number);

                                let new_anchor = leaf.view_number;
                                if new_anchor >= anchor_view {
                                    anchor_view = leaf.view_number;
                                }
                            }

                            if let Some(size) = block_size {
                                total_transactions += size;
                            }

                            num_successful_commits += leaf_chain.len();
                            if num_successful_commits >= rounds {
                                break;
                            }

                            if leaf_chain.len() > 1 {
                                warn!("Leaf chain is greater than 1 with len {}", leaf_chain.len());
                            }
                            // when we make progress, submit new events
                        }
                        EventType::ReplicaViewTimeout { view_number } => {
                            warn!("Timed out as a replicas in view {:?}", view_number);
                        }
                        EventType::NextLeaderViewTimeout { view_number } => {
                            warn!("Timed out as the next leader in view {:?}", view_number);
                        }
                        EventType::ViewFinished { view_number } => {
                            if *view_number > round {
                                round = *view_number;
                                info!("view finished: {:?}", view_number);
                                for _ in 0..transactions_per_round {
                                    if node_index >= total_nodes_u64 - 10 {
                                        let txn = txns.pop_front().unwrap();

                                        debug!("Submitting txn on round {}", round);

                                        let result = context.submit_transaction(txn).await;

                                        if result.is_err() {
                                            error! (
                                            "Could not send transaction to web server on round {}",
                                            round
                                        )
                                        }
                                    }
                                }
                            }
                        }
                        _ => unimplemented!(),
                    }
                }
            }

            round += 1;
        }

        // Output run results
        let total_time_elapsed = start.elapsed();
        error!("{rounds} rounds completed in {total_time_elapsed:?} - Total transactions committed: {total_transactions} - Total commitments: {num_successful_commits}");
    }

    /// Returns the da network for this run
    fn get_da_network(&self) -> DANETWORK;

    /// Returns the quorum network for this run
    fn get_quorum_network(&self) -> QUORUMNETWORK;

    ///Returns view sync network for this run
    fn get_view_sync_network(&self) -> VIEWSYNCNETWORK;

    /// Returns the config for this run
    fn get_config(
        &self,
    ) -> NetworkConfig<
        TYPES::SignatureKey,
        <TYPES::SignatureKey as SignatureKey>::StakeTableEntry,
        TYPES::ElectionConfigType,
    >;
}

// WEB SERVER

/// Represents a web server-based run
pub struct WebServerDARun<
    TYPES: NodeType,
    I: NodeImplementation<TYPES>,
    MEMBERSHIP: Membership<TYPES>,
> {
    config: NetworkConfig<
        TYPES::SignatureKey,
        <TYPES::SignatureKey as SignatureKey>::StakeTableEntry,
        TYPES::ElectionConfigType,
    >,
    quorum_network: WebCommChannel<TYPES, I, MEMBERSHIP>,
    da_network: WebCommChannel<TYPES, I, MEMBERSHIP>,
    view_sync_network: WebCommChannel<TYPES, I, MEMBERSHIP>,
}

#[async_trait]
impl<
        TYPES: NodeType,
        MEMBERSHIP: Membership<TYPES> + Debug,
        NODE: NodeImplementation<
            TYPES,
            Leaf = SequencingLeaf<TYPES>,
            Exchanges = SequencingExchanges<
                TYPES,
                Message<TYPES, NODE>,
                QuorumExchange<
                    TYPES,
                    SequencingLeaf<TYPES>,
                    QuorumProposal<TYPES, SequencingLeaf<TYPES>>,
                    MEMBERSHIP,
                    WebCommChannel<TYPES, NODE, MEMBERSHIP>,
                    Message<TYPES, NODE>,
                >,
                CommitteeExchange<
                    TYPES,
                    MEMBERSHIP,
                    WebCommChannel<TYPES, NODE, MEMBERSHIP>,
                    Message<TYPES, NODE>,
                >,
                ViewSyncExchange<
                    TYPES,
                    ViewSyncCertificate<TYPES>,
                    MEMBERSHIP,
                    WebCommChannel<TYPES, NODE, MEMBERSHIP>,
                    Message<TYPES, NODE>,
                >,
            >,
            Storage = MemoryStorage<TYPES, SequencingLeaf<TYPES>>,
            ConsensusMessage = SequencingMessage<TYPES, NODE>,
        >,
    >
    RunDA<
        TYPES,
        MEMBERSHIP,
        WebCommChannel<TYPES, NODE, MEMBERSHIP>,
        WebCommChannel<TYPES, NODE, MEMBERSHIP>,
        WebCommChannel<TYPES, NODE, MEMBERSHIP>,
        NODE,
    > for WebServerDARun<TYPES, NODE, MEMBERSHIP>
where
    <TYPES as NodeType>::StateType: TestableState,
    <TYPES as NodeType>::BlockType: TestableBlock,
    SequencingLeaf<TYPES>: TestableLeaf,
    Self: Sync,
{
    async fn initialize_networking(
        config: NetworkConfig<
            TYPES::SignatureKey,
            <TYPES::SignatureKey as SignatureKey>::StakeTableEntry,
            TYPES::ElectionConfigType,
        >,
    ) -> WebServerDARun<TYPES, NODE, MEMBERSHIP> {
        // Generate our own key
        let (pub_key, _priv_key) =
            <<TYPES as NodeType>::SignatureKey as SignatureKey>::generated_from_seed_indexed(
                config.seed,
                config.node_index,
            );

        // Get the configuration for the web server
        let WebServerConfig {
            host,
            port,
            wait_between_polls,
        }: WebServerConfig = config.clone().web_server_config.unwrap();

        let underlying_quorum_network = WebServerNetwork::create(
            &host.to_string(),
            port,
            wait_between_polls,
            pub_key.clone(),
            false,
        );

        // Create the network
        let quorum_network: WebCommChannel<TYPES, NODE, MEMBERSHIP> =
            WebCommChannel::new(underlying_quorum_network.clone().into());

        let view_sync_network: WebCommChannel<TYPES, NODE, MEMBERSHIP> =
            WebCommChannel::new(underlying_quorum_network.into());

        let WebServerConfig {
            host,
            port,
            wait_between_polls,
        }: WebServerConfig = config.clone().da_web_server_config.unwrap();

        // Each node runs the DA network so that leaders have access to transactions and DA votes
        let da_network: WebCommChannel<TYPES, NODE, MEMBERSHIP> = WebCommChannel::new(
            WebServerNetwork::create(&host.to_string(), port, wait_between_polls, pub_key, true)
                .into(),
        );

        WebServerDARun {
            config,
            quorum_network,
            da_network,
            view_sync_network,
        }
    }

    fn get_da_network(&self) -> WebCommChannel<TYPES, NODE, MEMBERSHIP> {
        self.da_network.clone()
    }

    fn get_quorum_network(&self) -> WebCommChannel<TYPES, NODE, MEMBERSHIP> {
        self.quorum_network.clone()
    }

    fn get_view_sync_network(&self) -> WebCommChannel<TYPES, NODE, MEMBERSHIP> {
        self.view_sync_network.clone()
    }

    fn get_config(
        &self,
    ) -> NetworkConfig<
        TYPES::SignatureKey,
        <TYPES::SignatureKey as SignatureKey>::StakeTableEntry,
        TYPES::ElectionConfigType,
    > {
        self.config.clone()
    }
}

// Libp2p

/// Represents a libp2p-based run
pub struct Libp2pDARun<TYPES: NodeType, I: NodeImplementation<TYPES>, MEMBERSHIP: Membership<TYPES>>
{
    config: NetworkConfig<
        TYPES::SignatureKey,
        <TYPES::SignatureKey as SignatureKey>::StakeTableEntry,
        TYPES::ElectionConfigType,
    >,
    quorum_network: Libp2pCommChannel<TYPES, I, MEMBERSHIP>,
    da_network: Libp2pCommChannel<TYPES, I, MEMBERSHIP>,
    view_sync_network: Libp2pCommChannel<TYPES, I, MEMBERSHIP>,
}

#[async_trait]
impl<
        TYPES: NodeType,
        MEMBERSHIP: Membership<TYPES> + Debug,
        NODE: NodeImplementation<
            TYPES,
            Leaf = SequencingLeaf<TYPES>,
            Exchanges = SequencingExchanges<
                TYPES,
                Message<TYPES, NODE>,
                QuorumExchange<
                    TYPES,
                    SequencingLeaf<TYPES>,
                    QuorumProposal<TYPES, SequencingLeaf<TYPES>>,
                    MEMBERSHIP,
                    Libp2pCommChannel<TYPES, NODE, MEMBERSHIP>,
                    Message<TYPES, NODE>,
                >,
                CommitteeExchange<
                    TYPES,
                    MEMBERSHIP,
                    Libp2pCommChannel<TYPES, NODE, MEMBERSHIP>,
                    Message<TYPES, NODE>,
                >,
                ViewSyncExchange<
                    TYPES,
                    ViewSyncCertificate<TYPES>,
                    MEMBERSHIP,
                    Libp2pCommChannel<TYPES, NODE, MEMBERSHIP>,
                    Message<TYPES, NODE>,
                >,
            >,
            Storage = MemoryStorage<TYPES, SequencingLeaf<TYPES>>,
            ConsensusMessage = SequencingMessage<TYPES, NODE>,
        >,
    >
    RunDA<
        TYPES,
        MEMBERSHIP,
        Libp2pCommChannel<TYPES, NODE, MEMBERSHIP>,
        Libp2pCommChannel<TYPES, NODE, MEMBERSHIP>,
        Libp2pCommChannel<TYPES, NODE, MEMBERSHIP>,
        NODE,
    > for Libp2pDARun<TYPES, NODE, MEMBERSHIP>
where
    <TYPES as NodeType>::StateType: TestableState,
    <TYPES as NodeType>::BlockType: TestableBlock,
    SequencingLeaf<TYPES>: TestableLeaf,
    Self: Sync,
{
    async fn initialize_networking(
        config: NetworkConfig<
            TYPES::SignatureKey,
            <TYPES::SignatureKey as SignatureKey>::StakeTableEntry,
            TYPES::ElectionConfigType,
        >,
    ) -> Libp2pDARun<TYPES, NODE, MEMBERSHIP> {
        let (pubkey, _privkey) =
            <<TYPES as NodeType>::SignatureKey as SignatureKey>::generated_from_seed_indexed(
                config.seed,
                config.node_index,
            );
        let mut config = config;
        let libp2p_config = config
            .libp2p_config
            .take()
            .expect("Configuration is not for a Libp2p network");
        let bs_len = libp2p_config.bootstrap_nodes.len();
        let bootstrap_nodes: Vec<(PeerId, Multiaddr)> = libp2p_config
            .bootstrap_nodes
            .iter()
            .map(|(addr, pair)| {
                let kp = Keypair::from_protobuf_encoding(pair).unwrap();
                let peer_id = PeerId::from_public_key(&kp.public());
                let multiaddr =
                    Multiaddr::from_str(&format!("/ip4/{}/udp/{}/quic-v1", addr.ip(), addr.port()))
                        .unwrap();
                (peer_id, multiaddr)
            })
            .collect();
        let identity = libp2p_generate_indexed_identity(config.seed, config.node_index);
        let node_type = if (config.node_index as usize) < bs_len {
            NetworkNodeType::Bootstrap
        } else {
            NetworkNodeType::Regular
        };
        let node_index = config.node_index;
        let port_index = match libp2p_config.index_ports {
            true => node_index,
            false => 0,
        };
        let bound_addr: Multiaddr = format!(
            "/{}/{}/udp/{}/quic-v1",
            if libp2p_config.public_ip.is_ipv4() {
                "ip4"
            } else {
                "ip6"
            },
            libp2p_config.public_ip,
            libp2p_config.base_port as u64 + port_index
        )
        .parse()
        .unwrap();

        // generate network
        let mut config_builder = NetworkNodeConfigBuilder::default();
        assert!(config.config.total_nodes.get() > 2);
        let replicated_nodes = NonZeroUsize::new(config.config.total_nodes.get() - 2).unwrap();
        config_builder.replication_factor(replicated_nodes);
        config_builder.identity(identity.clone());

        config_builder.bound_addr(Some(bound_addr.clone()));

        let to_connect_addrs = bootstrap_nodes
            .iter()
            .map(|(peer_id, multiaddr)| (Some(*peer_id), multiaddr.clone()))
            .collect();

        config_builder.to_connect_addrs(to_connect_addrs);

        let mesh_params =
        // NOTE I'm arbitrarily choosing these.
        match node_type {
            NetworkNodeType::Bootstrap => MeshParams {
                mesh_n_high: libp2p_config.bootstrap_mesh_n_high,
                mesh_n_low: libp2p_config.bootstrap_mesh_n_low,
                mesh_outbound_min: libp2p_config.bootstrap_mesh_outbound_min,
                mesh_n: libp2p_config.bootstrap_mesh_n,
            },
            NetworkNodeType::Regular => MeshParams {
                mesh_n_high: libp2p_config.mesh_n_high,
                mesh_n_low: libp2p_config.mesh_n_low,
                mesh_outbound_min: libp2p_config.mesh_outbound_min,
                mesh_n: libp2p_config.mesh_n,
            },
            NetworkNodeType::Conductor => unreachable!(),
        };
        config_builder.mesh_params(Some(mesh_params));

        let mut all_keys = BTreeSet::new();
        let mut da_keys = BTreeSet::new();
        for i in 0..config.config.total_nodes.get() as u64 {
            let privkey = TYPES::SignatureKey::generated_from_seed_indexed([0u8; 32], i).1;
            let pubkey = TYPES::SignatureKey::from_private(&privkey);
            if i < config.config.da_committee_size as u64 {
                da_keys.insert(pubkey.clone());
            }
            all_keys.insert(pubkey);
        }

        let node_config = config_builder.build().unwrap();
        let underlying_quorum_network = Libp2pNetwork::new(
            NoMetrics::boxed(),
            node_config,
            pubkey.clone(),
            Arc::new(RwLock::new(
                bootstrap_nodes
                    .iter()
                    .map(|(peer_id, addr)| (Some(*peer_id), addr.clone()))
                    .collect(),
            )),
            bs_len,
            config.node_index as usize,
            // NOTE: this introduces an invariant that the keys are assigned using this indexed
            // function
            all_keys,
            da_keys,
        )
        .await
        .unwrap();

        underlying_quorum_network.wait_for_ready().await;

        // Create the network
        let quorum_network: Libp2pCommChannel<TYPES, NODE, MEMBERSHIP> =
            Libp2pCommChannel::new(underlying_quorum_network.clone().into());

        let view_sync_network: Libp2pCommChannel<TYPES, NODE, MEMBERSHIP> =
            Libp2pCommChannel::new(underlying_quorum_network.clone().into());

        let da_network: Libp2pCommChannel<TYPES, NODE, MEMBERSHIP> =
            Libp2pCommChannel::new(underlying_quorum_network.clone().into());

        Libp2pDARun {
            config,
            quorum_network,
            da_network,
            view_sync_network,
        }
    }

    fn get_da_network(&self) -> Libp2pCommChannel<TYPES, NODE, MEMBERSHIP> {
        self.da_network.clone()
    }

    fn get_quorum_network(&self) -> Libp2pCommChannel<TYPES, NODE, MEMBERSHIP> {
        self.quorum_network.clone()
    }

    fn get_view_sync_network(&self) -> Libp2pCommChannel<TYPES, NODE, MEMBERSHIP> {
        self.view_sync_network.clone()
    }

    fn get_config(
        &self,
    ) -> NetworkConfig<
        TYPES::SignatureKey,
        <TYPES::SignatureKey as SignatureKey>::StakeTableEntry,
        TYPES::ElectionConfigType,
    > {
        self.config.clone()
    }
}

/// Main entry point for validators
pub async fn main_entry_point<
    TYPES: NodeType,
    MEMBERSHIP: Membership<TYPES> + Debug,
    DANETWORK: CommunicationChannel<TYPES, Message<TYPES, NODE>, MEMBERSHIP> + Debug,
    QUORUMNETWORK: CommunicationChannel<TYPES, Message<TYPES, NODE>, MEMBERSHIP> + Debug,
    VIEWSYNCNETWORK: CommunicationChannel<TYPES, Message<TYPES, NODE>, MEMBERSHIP> + Debug,
    NODE: NodeImplementation<
        TYPES,
        Leaf = SequencingLeaf<TYPES>,
        Exchanges = SequencingExchanges<
            TYPES,
            Message<TYPES, NODE>,
            QuorumExchange<
                TYPES,
                SequencingLeaf<TYPES>,
                QuorumProposal<TYPES, SequencingLeaf<TYPES>>,
                MEMBERSHIP,
                QUORUMNETWORK,
                Message<TYPES, NODE>,
            >,
            CommitteeExchange<TYPES, MEMBERSHIP, DANETWORK, Message<TYPES, NODE>>,
            ViewSyncExchange<
                TYPES,
                ViewSyncCertificate<TYPES>,
                MEMBERSHIP,
                VIEWSYNCNETWORK,
                Message<TYPES, NODE>,
            >,
        >,
        Storage = MemoryStorage<TYPES, SequencingLeaf<TYPES>>,
        ConsensusMessage = SequencingMessage<TYPES, NODE>,
    >,
    RUNDA: RunDA<TYPES, MEMBERSHIP, DANETWORK, QUORUMNETWORK, VIEWSYNCNETWORK, NODE>,
>(
    args: ValidatorArgs,
) where
    <TYPES as NodeType>::StateType: TestableState,
    <TYPES as NodeType>::BlockType: TestableBlock,
    SequencingLeaf<TYPES>: TestableLeaf,
{
    setup_logging();
    setup_backtrace();

    info!("Starting validator");

    let orchestrator_client: OrchestratorClient =
        OrchestratorClient::connect_to_orchestrator(args.clone()).await;

    // Identify with the orchestrator
    let public_ip = match args.public_ip {
        Some(ip) => ip,
        None => IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    };
    info!(
        "Identifying with orchestrator using IP address {}",
        public_ip.to_string()
    );
    let node_index: u16 = orchestrator_client
        .identify_with_orchestrator(public_ip.to_string())
        .await;
    info!("Finished identifying; our node index is {node_index}");
    info!("Getting config from orchestrator");

    let mut run_config = orchestrator_client
        .get_config_from_orchestrator::<TYPES>(node_index)
        .await;

    run_config.node_index = node_index.into();
    //run_config.libp2p_config.as_mut().unwrap().public_ip = args.public_ip.unwrap();

    info!("Initializing networking");
    let run = RUNDA::initialize_networking(run_config.clone()).await;
    let hotshot = run.initialize_state_and_hotshot().await;

    info!("Waiting for start command from orchestrator");
    orchestrator_client
        .wait_for_all_nodes_ready(run_config.clone().node_index)
        .await;

    info!("All nodes are ready!  Starting HotShot");
    run.run_hotshot(hotshot).await;
}

pub fn libp2p_generate_indexed_identity(seed: [u8; 32], index: u64) -> Keypair {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed);
    hasher.update(&index.to_le_bytes());
    let new_seed = *hasher.finalize().as_bytes();
    let sk_bytes = SecretKey::try_from_bytes(new_seed).unwrap();
    <ed25519::Keypair as From<SecretKey>>::from(sk_bytes).into()
}
