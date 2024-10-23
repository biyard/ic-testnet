use anyhow::Result;
use ic_config::artifact_pool::LMDBConfig;
use ic_config::embedders::FeatureFlags;
use ic_config::flag_status::FlagStatus;
use ic_config::logger::Level;
use ic_config::{
    adapters::AdaptersConfig, artifact_pool::ArtifactPoolTomlConfig, crypto::CryptoConfig,
    http_handler::Config as HttpHandlerConfig, logger::Config as LoggerConfig,
    registry_client::Config as RegistryClientConfig, state_manager::Config as StateManagerConfig,
    transport::TransportConfig, ConfigOptional as ReplicaConfig,
};
use ic_config::{
    embedders::Config as EmbeddersConfig, execution_environment::Config as HypervisorConfig,
};
use ic_logger::{info, new_replica_logger_from_config, no_op_logger};
use ic_prep_lib::subnet_configuration::{constants, SubnetIndex};
use ic_prep_lib::{
    internet_computer::{IcConfig, TopologyConfig},
    node::{NodeConfiguration, NodeIndex},
    subnet_configuration::{SubnetConfig, SubnetRunningState},
};
use ic_protobuf::types::v1::ConsensusMessage;
use ic_registry_provisional_whitelist::ProvisionalWhitelist;
use ic_registry_subnet_features::SubnetFeatures;
use ic_registry_subnet_type::SubnetType;
use ic_types::{Cycles, Height, ReplicaVersion};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::{collections::BTreeMap, net::SocketAddr};
use std::{env, fs};
use std::{io, str::FromStr};

const NODE_INDEX: NodeIndex = 100;

fn write_replica_config(node_index: NodeIndex, addr: SocketAddr) -> Result<()> {
    let logger_config = LoggerConfig {
        level: Level::Trace,
        ..LoggerConfig::default()
    };
    let (log, _async_log_guard) = new_replica_logger_from_config(&logger_config);
    let mut node_dir = env::current_dir()?;
    node_dir.push("tmp");

    let config_path = node_dir.join(format!("ic-{}.json5", node_index));

    info!(log, "Initialize replica configuration {:?}", config_path);

    let replica_config = build_replica_config(node_index, addr)?;

    // assemble config
    let config_json = serde_json::to_string(&replica_config)?;
    std::fs::write(config_path.clone(), config_json.into_bytes())?;
    Ok(())
}

fn lmdb() {
    let conf = LMDBConfig {
        persistent_pool_validated_persistent_db_path: PathBuf::from("ic_consensus_pool"),
    };
    let log = no_op_logger();
    use ic_interfaces::consensus_pool::PoolSection;

    let pool = ic_artifact_pool::lmdb_pool::PersistentHeightIndexedPool::new_consensus_pool(
        conf, true, log,
    );
    let range = pool.block_proposal().height_range().unwrap();
    println!("{range:?}")
}

fn main() -> Result<()> {
    let mut node_dir = env::current_dir()?;
    node_dir.push("tmp");

    let nodes: Vec<String> = option_env!("NODES")
        .unwrap_or("10.5.0.10 10.5.0.11 10.5.0.12 10.5.0.13")
        .split(" ")
        .map(|s| s.to_string())
        .collect();

    let bindings: Vec<(String, String, Option<u64>)> = nodes
        .iter()
        .map(|node| (format!("{}:4100", node), format!("{}:4101", node), Some(0)))
        .collect::<Vec<_>>();

    let mut unassinged_nodes: BTreeMap<NodeIndex, NodeConfiguration> = BTreeMap::new();
    let mut state_dir = env::current_dir()?;
    state_dir.push(format!("tmp"));
    state_dir.push(format!("state"));

    if !state_dir.is_dir() {
        fs::create_dir_all(state_dir.clone())?;
    }

    let mut subnets: BTreeMap<SubnetIndex, BTreeMap<NodeIndex, NodeConfiguration>> =
        BTreeMap::new();

    for (i, binding) in bindings.iter().enumerate() {
        let node_index = NODE_INDEX + i as NodeIndex;
        let addr = binding.0.parse()?;
        write_replica_config(node_index, addr)?;

        match binding.2 {
            Some(subnet_id) => {
                let subnet = subnets.entry(subnet_id).or_insert(BTreeMap::new());
                subnet.insert(
                    node_index,
                    NodeConfiguration {
                        xnet_api: SocketAddr::from_str(&binding.1).unwrap(),
                        public_api: addr,
                        node_operator_principal_id: None,
                        secret_key_store: None,
                    },
                );
            }
            None => {
                unassinged_nodes.insert(
                    node_index,
                    NodeConfiguration {
                        xnet_api: SocketAddr::from_str(&binding.1).unwrap(),
                        public_api: addr,
                        node_operator_principal_id: None,
                        secret_key_store: None,
                    },
                );
            }
        }
    }

    let mut topology_config = TopologyConfig::default();
    for (subnet_id, subnet_nodes) in subnets {
        let conf = SubnetConfig::new(
            subnet_id,
            subnet_nodes.clone(),
            ReplicaVersion::default(),
            None,
            Some(5000),                                  // max_ingress_messages_per_block
            Some(constants::MAX_BLOCK_PAYLOAD_SIZE * 5), //max_block_payload_size 5 * 4MB
            None,                                        //config.unit_delay,
            None,                                        // config.initial_notary_delay,
            None,                                        // config.dkg_interval_length,
            None,
            match subnet_id {
                // 0 => SubnetType::System,
                _ => SubnetType::Application,
            },
            None,
            None,
            None,
            Some(SubnetFeatures::default()),
            None, // chain_key_config,
            None,
            vec![],
            vec![],
            SubnetRunningState::default(),
            Some(0),
        );

        #[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
        pub struct SubnetConfigJson {
            id: u64,
            nodes: BTreeMap<NodeIndex, NodeConfiguration>,
            pub max_ingress_bytes_per_message: u64,
            pub max_ingress_messages_per_block: u64,
            pub max_block_payload_size: u64,
            pub max_instructions_per_message: u64,
            pub max_instructions_per_round: u64,
            pub max_instructions_per_install_code: u64,
            pub max_number_of_canisters: u64,
            pub initial_height: u64,
        }

        impl From<SubnetConfig> for SubnetConfigJson {
            fn from(conf: SubnetConfig) -> Self {
                SubnetConfigJson {
                    id: conf.subnet_index,
                    nodes: conf.membership.clone(),
                    max_ingress_bytes_per_message: conf.max_ingress_bytes_per_message,
                    max_ingress_messages_per_block: conf.max_ingress_messages_per_block,
                    max_block_payload_size: conf.max_block_payload_size,
                    max_instructions_per_message: conf.max_instructions_per_message,
                    max_instructions_per_round: conf.max_instructions_per_round,
                    max_instructions_per_install_code: conf.max_instructions_per_install_code,
                    max_number_of_canisters: conf.max_number_of_canisters,
                    initial_height: conf.initial_height,
                }
            }
        }

        let config_path = node_dir.join(format!("subnet-{}.json", subnet_id));
        let config_json = serde_json::to_string(&SubnetConfigJson::from(conf.clone()))?;
        std::fs::write(config_path.clone(), config_json.into_bytes())?;
        topology_config.insert_subnet(subnet_id, conf.clone());
    }

    for (idx, nc) in unassinged_nodes {
        topology_config.insert_unassigned_node(idx, nc)
    }

    let mut ic_config = IcConfig::new(
        /* target_dir= */ state_dir.as_path(),
        topology_config,
        ReplicaVersion::default(),
        /* generate_subnet_records= */ true, // see note above
        /* nns_subnet_index= */ Some(0),
        /* release_package_url= */ None,
        /* release_package_sha256_hex */ None,
        Some(ProvisionalWhitelist::All),
        None,
        None,
        /* ssh_readonly_access_to_unassigned_nodes */ vec![],
    );

    ic_config.set_use_specified_ids_allocation_range(false);

    ic_config.initialize()?;

    Ok(())
}

fn build_replica_config(
    node_index: NodeIndex,
    http_listen_addr: SocketAddr,
) -> io::Result<ReplicaConfig> {
    let mut state_dir = match env::var("BASE_DIR") {
        Ok(dir) => PathBuf::from(dir),
        _ => env::current_dir()?,
    };
    state_dir.push(format!("state-{}", node_index));

    let node_dir = state_dir.join(format!("node-{}", node_index));
    let artifact_pool_dir = node_dir.join("ic_consensus_pool");
    let crypto_root = node_dir.join("crypto");
    let state_manager_root = node_dir.join("state");
    let registry_local_store_path = state_dir.join("ic_registry_local_store");

    let state_manager = Some(StateManagerConfig::new(state_manager_root.clone()));
    let http_handler = Some(HttpHandlerConfig {
        listen_addr: http_listen_addr,
        http_max_concurrent_streams: 10000,
        max_read_state_concurrent_requests: 2000,
        max_status_concurrent_requests: 2000,
        max_catch_up_package_concurrent_requests: 2000,
        max_dashboard_concurrent_requests: 100,
        max_call_concurrent_requests: 5000,
        max_query_concurrent_requests: 5000,
        max_pprof_concurrent_requests: 5,
        ..Default::default()
    });

    let mut artifact_pool_cfg = ArtifactPoolTomlConfig::new(artifact_pool_dir.clone(), None);
    // artifact_pool.rs picks "lmdb" if None here
    artifact_pool_cfg.consensus_pool_backend.clone_from(&None);
    let artifact_pool = Some(artifact_pool_cfg);

    let crypto = Some(CryptoConfig::new(crypto_root.clone()));
    let registry_client = Some(RegistryClientConfig {
        local_store: registry_local_store_path.clone(),
    });
    let logger_config = LoggerConfig {
        level: ic_config::logger::Level::Info,
        ..LoggerConfig::default()
    };
    let logger = Some(logger_config);

    let transport = Some(TransportConfig {
        node_ip: "0.0.0.0".to_string(),
        listening_port: 4100,
        send_queue_size: 1024,
        ..Default::default()
    });

    let hypervisor = Some(HypervisorConfig {
        canister_sandboxing_flag: FlagStatus::Disabled,
        deterministic_time_slicing: FlagStatus::Disabled,
        create_funds_whitelist: "*".to_string(),

        embedders_config: EmbeddersConfig {
            feature_flags: FeatureFlags {
                rate_limiting_of_debug_prints: FlagStatus::Disabled,
                best_effort_responses: FlagStatus::Enabled,
                wasm64: FlagStatus::Enabled,
                ..FeatureFlags::default()
            },
            ..EmbeddersConfig::default()
        },
        rate_limiting_of_heap_delta: FlagStatus::Disabled,
        rate_limiting_of_instructions: FlagStatus::Disabled,
        canister_snapshots: FlagStatus::Enabled,
        query_stats_epoch_length: 60,
        default_provisional_cycles_balance: Cycles::new(18_446_744_073_709_551_616),

        ..HypervisorConfig::default()
    });

    let adapters_config = Some(AdaptersConfig {
        https_outcalls_uds_path: Some(node_dir.join("https_outcalls")),
        ..AdaptersConfig::default()
    });

    Ok(ReplicaConfig {
        registry_client,
        transport,
        state_manager,
        hypervisor,
        http_handler,
        metrics: None,
        artifact_pool,
        crypto,
        logger,
        adapters_config,
        ..ReplicaConfig::default()
    })
}
