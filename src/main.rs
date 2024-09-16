use anyhow::Result;
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
use ic_logger::{info, new_replica_logger_from_config};
use ic_prep_lib::subnet_configuration::SubnetIndex;
use ic_prep_lib::{
    internet_computer::{IcConfig, TopologyConfig},
    node::{NodeConfiguration, NodeIndex},
    subnet_configuration::{SubnetConfig, SubnetRunningState},
};
use ic_registry_provisional_whitelist::ProvisionalWhitelist;
use ic_registry_subnet_features::SubnetFeatures;
use ic_registry_subnet_type::SubnetType;
use ic_types::ReplicaVersion;
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
    let mut node_dir = env::current_dir()?; // PathBuf::from("/workdir");
    node_dir.push("tmp");

    let config_path = node_dir.join(format!("ic-{}.json5", node_index));

    info!(log, "Initialize replica configuration {:?}", config_path);

    let mut replica_config = build_replica_config(node_index, addr)?;
    replica_config
        .hypervisor
        .as_mut()
        .unwrap()
        .deterministic_time_slicing = FlagStatus::Disabled;

    // assemble config
    let config_json = serde_json::to_string(&replica_config)?;
    std::fs::write(config_path.clone(), config_json.into_bytes())?;
    Ok(())
}

// use ic_system_test_driver::driver::ic::{InternetComputer, Subnet};

// /// The following setup function demonstrates how to create more than one
// /// Internet Computer instances within a setup function.
// pub fn setup_two_ics() {
//     let mut ic = InternetComputer::new().add_subnet(Subnet::new(SubnetType::System).add_nodes(4));
//     setup_and_start(&mut ic, &test_env).expect("Could not start no-name IC");
// }

fn main() -> Result<()> {
    let bindings = [
        ("127.0.1.1:4100", "127.0.1.1:0", Some(0)),
        ("127.0.2.1:4100", "127.0.2.1:0", Some(1)),
        ("127.0.3.1:4100", "127.0.3.1:0", Some(1)),
        ("127.0.4.1:4100", "127.0.4.1:0", Some(1)),
    ];

    let mut unassinged_nodes: BTreeMap<NodeIndex, NodeConfiguration> = BTreeMap::new();
    let mut state_dir = match env::var("BASE_DIR") {
        Ok(dir) => PathBuf::from(dir),
        _ => env::current_dir()?,
    };
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
                        xnet_api: SocketAddr::from_str(binding.1).unwrap(),
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
                        xnet_api: SocketAddr::from_str(binding.1).unwrap(),
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
        topology_config.insert_subnet(
            subnet_id,
            SubnetConfig::new(
                subnet_id,
                subnet_nodes.clone(),
                ReplicaVersion::default(),
                None,
                None,
                None,
                None, //config.unit_delay,
                None, // config.initial_notary_delay,
                None, // config.dkg_interval_length,
                None,
                match subnet_id {
                    0 => SubnetType::System,
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
                Some(1),
            ),
        );
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

    ic_config.set_use_specified_ids_allocation_range(true);

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
        level: ic_config::logger::Level::Trace,
        ..LoggerConfig::default()
    };
    let logger = Some(logger_config);

    let transport = Some(TransportConfig {
        node_ip: "0.0.0.0".to_string(),
        listening_port: 0,
        send_queue_size: 1024,
        ..Default::default()
    });

    let hypervisor = Some(HypervisorConfig {
        canister_sandboxing_flag: FlagStatus::Disabled,
        deterministic_time_slicing: FlagStatus::Disabled,

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
        ..HypervisorConfig::default()
    });

    let adapters_config = Some(AdaptersConfig {
        https_outcalls_uds_path: None,
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
