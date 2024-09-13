use anyhow::Result;
use clap::Parser;
use ic_config::embedders::FeatureFlags;
use ic_config::flag_status::FlagStatus;
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
use ic_management_canister_types::{EcdsaKeyId, MasterPublicKeyId};
use ic_prep_lib::{
    internet_computer::{IcConfig, TopologyConfig},
    node::{NodeConfiguration, NodeIndex},
    subnet_configuration::{SubnetConfig, SubnetRunningState},
};
use ic_registry_provisional_whitelist::ProvisionalWhitelist;
use ic_registry_subnet_features::SubnetFeatures;
use ic_registry_subnet_type::SubnetType;
use ic_types::{Height, ReplicaVersion};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};
use std::{env, fs};
use std::{io, path::PathBuf, str::FromStr};

const NODE_INDEX: NodeIndex = 100;
const SUBNET_ID: u64 = 0;

fn write_replica_config(args: CliArgs, node_index: NodeIndex) -> Result<ValidatedConfig> {
    let config = args.validate(node_index)?;
    let logger_config = LoggerConfig {
        level: config.log_level,
        ..LoggerConfig::default()
    };
    let (log, _async_log_guard) = new_replica_logger_from_config(&logger_config);

    info!(log, "ic-starter. Configuration: {:?}", config);

    let config_path = config.state_dir.join(format!("ic-{}.json5", node_index));

    info!(log, "Initialize replica configuration {:?}", config_path);

    let mut replica_config = config.build_replica_config();
    replica_config
        .hypervisor
        .as_mut()
        .unwrap()
        .deterministic_time_slicing = FlagStatus::Disabled;

    // assemble config
    let config_json = serde_json::to_string(&replica_config).unwrap();
    std::fs::write(config_path.clone(), config_json.into_bytes()).unwrap();

    Ok(config)
}

fn main() -> Result<()> {
    let mut args = CliArgs::parse();
    args.provisional_whitelist = Some("*".to_string());
    args.use_specified_ids_allocation_range = true;
    args.subnet_type = Some("application".to_string());
    let bindings = [
        ("127.0.0.1:8080", "127.0.0.1:8081"),
        ("127.0.0.1:9080", "127.0.0.1:9081"),
        // ("127.0.0.1:10080", "127.0.0.1:10081"),
        // ("127.0.0.1:11080", "127.0.0.1:11081"),
    ];

    // let bindings = [
    //     ("10.5.0.10:8080", "10.5.0.10:8081"),
    //     ("10.5.0.11:8080", "10.5.0.11:8081"),
    //     ("10.5.0.12:8080", "10.5.0.12:8081"),
    //     ("10.5.0.13:8080", "10.5.0.13:8081"),
    // ];
    let mut subnet_nodes: BTreeMap<NodeIndex, NodeConfiguration> = BTreeMap::new();
    let mut configs = vec![];

    for (i, binding) in bindings.iter().enumerate() {
        let args = CliArgs {
            http_listen_addr: Some(binding.0.parse().unwrap()),
            http_port_file: None,
            canister_http_uds_path: Some(format!("tmp/ic-canister-http-adapter-{i}.sock").into()),
            ..args.clone()
        };
        let node_index = NODE_INDEX + i as NodeIndex;

        let config = write_replica_config(args, node_index)?;

        subnet_nodes.insert(
            node_index,
            NodeConfiguration {
                xnet_api: SocketAddr::from_str(binding.1).unwrap(),
                public_api: config.http_listen_addr,
                node_operator_principal_id: None,
                secret_key_store: None,
            },
        );
        configs.push(config);
    }

    let config = configs.first().unwrap();

    let mut topology_config = TopologyConfig::default();
    topology_config.insert_subnet(
        SUBNET_ID,
        SubnetConfig::new(
            SUBNET_ID,
            subnet_nodes.clone(),
            config.replica_version.clone(),
            None,
            None,
            None,
            config.unit_delay,
            config.initial_notary_delay,
            config.dkg_interval_length,
            None,
            config.subnet_type,
            None,
            None,
            None,
            Some(config.subnet_features),
            None, // chain_key_config,
            None,
            vec![],
            vec![],
            SubnetRunningState::default(),
            None,
        ),
    );

    // N.B. it is safe to generate subnet records here, we only skip this
    // step for a specific deployment case in ic-prep: when we want to deploy
    // nodes without assigning them to subnets

    let mut ic_config = IcConfig::new(
        /* target_dir= */ config.state_dir.as_path(),
        topology_config,
        config.replica_version.clone(),
        /* generate_subnet_records= */ true, // see note above
        /* nns_subnet_index= */ Some(0),
        /* release_package_url= */ None,
        /* release_package_sha256_hex */ None,
        Some(ProvisionalWhitelist::All),
        None,
        None,
        /* ssh_readonly_access_to_unassigned_nodes */ vec![],
    );

    ic_config.set_use_specified_ids_allocation_range(config.use_specified_ids_allocation_range);

    ic_config.initialize()?;

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Parser)]
#[clap(name = "ic-starter", about = "Starter.", version)]
struct CliArgs {
    /// Path the to replica binary.
    ///
    /// The replica binary will be built if not provided. In this case, it is
    /// expected that a config will be found for '--bin replica'. In other
    /// words, it is expected that the starter is invoked from the rs/
    /// directory.
    #[clap(long = "replica-path", parse(from_os_str))]
    replica_path: Option<PathBuf>,

    /// Version of the replica binary.
    #[clap(long, parse(try_from_str = ReplicaVersion::try_from))]
    replica_version: Option<ReplicaVersion>,

    /// Path to the cargo binary. Not optional because there is a default value.
    ///
    /// Unused if --replica-path is present
    #[clap(long = "cargo", default_value = "cargo")]
    cargo_bin: String,

    /// Options to pass to cargo, such as "--release". Not optional because
    /// there is a default value.
    ///
    /// Several options can be passed, with whitespaces inside the value. For
    /// instance: cargo run ic-starter -- '--cargo-opts=--release --quiet'
    ///
    /// Unused if --replica-path is present
    #[clap(long = "cargo-opts", default_value = "")]
    cargo_opts: String,

    /// Path to the directory containing all state for this replica. (default: a
    /// temp directory that will be deleted immediately when the replica
    /// stops).
    #[clap(long = "state-dir", parse(from_os_str))]
    state_dir: Option<PathBuf>,

    /// The http port of the public API.
    ///
    /// If not specified, and if --http-port-file is empty, then 8080 will be
    /// used.
    ///
    /// This argument is incompatible with --http-port-file.
    #[clap(long = "http-port")]
    http_port: Option<u16>,

    /// The http listening address of the public API.
    ///
    /// If not specified, and if --http-port-file is empty, then 127.0.0.1:8080
    /// will be used.
    #[clap(long = "http-listen-addr")]
    http_listen_addr: Option<SocketAddr>,

    /// The file where the chosen port of the public api will be written to.
    /// When this option is used, a free port will be chosen by the replica
    /// at start time.
    ///
    /// This argument is incompatible with --http-port.
    #[clap(long = "http-port-file", parse(from_os_str))]
    http_port_file: Option<PathBuf>,

    /// Arg to control whitelist for creating funds which is either set to "*"
    /// or "".
    #[clap(short = 'c', long = "create-funds-whitelist")]
    provisional_whitelist: Option<String>,

    /// Run replica and ic-starter with the provided log level. Default is Warning
    #[clap(long = "log-level",
                possible_values = &["critical", "error", "warning", "info", "debug", "trace"],
                ignore_case = true)]
    log_level: Option<String>,

    // /// Metrics port. Default is None, i.e. periodically dump metrics on stdout.
    // #[clap(long = "metrics-port")]
    // metrics_port: Option<u16>,

    // /// Metrics address. Use this in preference to metrics-port
    // #[clap(long = "metrics-addr")]
    // metrics_addr: Option<SocketAddr>,
    /// Unit delay for blockmaker (in milliseconds).
    /// If running integration tests locally (e.g. ic-ref-test),
    /// setting this to 100ms results in faster execution (and higher
    /// CPU consumption).
    #[clap(long = "unit-delay-millis")]
    unit_delay_millis: Option<u64>,

    /// Initial delay for notary (in milliseconds).
    /// If running integration tests locally (e.g. ic-ref-test),
    /// setting this to 100ms results in faster execution (and higher
    /// CPU consumption).
    #[clap(long = "initial-notary-delay-millis")]
    initial_notary_delay_millis: Option<u64>,

    /// DKG interval length (in number of blocks).
    #[clap(long = "dkg-interval-length")]
    dkg_interval_length: Option<u64>,

    /// The backend DB used by Consensus, can be rocksdb or lmdb.
    #[clap(long = "consensus-pool-backend",
                possible_values = &["lmdb", "rocksdb"])]
    consensus_pool_backend: Option<String>,

    /// Subnet features
    #[clap(long = "subnet-features",
        possible_values = &[
            "canister_sandboxing",
            "http_requests",
            "bitcoin_testnet",
            "bitcoin_testnet_syncing",
            "bitcoin_testnet_paused",
            "bitcoin_mainnet",
            "bitcoin_mainnet_syncing",
            "bitcoin_mainnet_paused",
            "bitcoin_regtest",
            "bitcoin_regtest_syncing",
            "bitcoin_regtest_paused",
        ],
        multiple_values(true))]
    subnet_features: Vec<String>,

    /// Enable ecdsa signature by assigning the given key id a freshly generated key.
    #[clap(long = "ecdsa-keyid")]
    ecdsa_keyid: Option<String>,

    /// Enable threshold signatures by assigning the given key id a freshly generated key. Key IDs have
    /// the form `schnorr:[algorithm]:[name]` for tSchnorr, and `ecdsa:[curve]:[name]` for tECDSA
    #[clap(long = "chain-key-ids")]
    chain_key_ids: Vec<String>,

    /// Subnet type
    #[clap(long = "subnet-type",
                possible_values = &["application", "verified_application", "system"])]
    subnet_type: Option<String>,

    /// Unix Domain Socket for Bitcoin testnet
    #[clap(long = "bitcoin-testnet-uds-path")]
    bitcoin_testnet_uds_path: Option<PathBuf>,

    /// Unix Domain Socket for canister http adapter
    #[clap(long = "canister-http-uds-path")]
    canister_http_uds_path: Option<PathBuf>,

    /// Whether or not to assign canister ID allocation range for specified IDs to subnet.
    /// Used only for local replicas.
    #[clap(long = "use-specified-ids-allocation-range")]
    use_specified_ids_allocation_range: bool,
}

impl CliArgs {
    fn validate(self, node_index: NodeIndex) -> io::Result<ValidatedConfig> {
        let replica_version = ReplicaVersion::default();
        let mut state_dir = env::current_dir()?; // PathBuf::from("/workdir");
        state_dir.push("tmp");
        state_dir.push(format!("state-{}", node_index));

        if !state_dir.is_dir() {
            fs::create_dir_all(state_dir.clone())?;
        }

        let node_dir = state_dir.join(format!("node-{}", node_index));

        if state_dir.metadata()?.permissions().readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("Cannot write state directory at: {:?}", state_dir),
            ));
        }

        let (http_listen_addr, http_port_file) =
            match (self.http_port, self.http_listen_addr, self.http_port_file) {
                (None, None, None) => Ok((
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080),
                    None,
                )),
                (None, None, Some(path)) => Ok((
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
                    Some(path),
                )),
                (None, Some(listen_addr), None) => Ok((listen_addr, None)),
                (None, Some(listen_addr), Some(path)) => Ok((listen_addr, Some(path))),
                (Some(port), None, None) => Ok((
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port),
                    None,
                )),
                (Some(_), None, Some(_)) => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Arguments --http-port and --http-port-file are incompatible.",
                )),
                (Some(_), Some(_), _) => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Arguments --http-port and --http-listen-addr are incompatible",
                )),
            }?;

        // check whether parent directory of port_file exists
        if let Some(http_port_file) = &http_port_file {
            if http_port_file
                .parent()
                .and_then(|p| if !p.is_dir() { None } else { Some(p) })
                .is_none()
            {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "Parent directory of http_port_file not found at",
                        // replica_path
                    ),
                ));
            }
        }

        // let metrics_port = self.metrics_port;
        // let mut metrics_addr = self.metrics_addr;

        // if metrics_addr.is_some() && metrics_port.is_some() {
        //     return Err(io::Error::new(
        //         io::ErrorKind::InvalidInput,
        //         "can't pass --metrics-addr and --metrics-port at the same time",
        //     ));
        // }

        // if let Some(port) = metrics_port {
        //     println!("--metrics-port is deprecated, use --metrics-addr instead");
        //     // If only --metrics-port is given then fallback to previous behaviour
        //     // of listening on 0.0.0.0. Note that this will trigger warning
        //     // popups on macOS.
        //     metrics_addr =
        //         Some(SocketAddrV4::new("0.0.0.0".parse().expect("can't fail"), port).into());
        // }

        let log_level = ic_config::logger::Level::Trace;

        let artifact_pool_dir = node_dir.join("ic_consensus_pool");
        let crypto_root = node_dir.join("crypto");
        let state_manager_root = node_dir.join("state");
        let registry_local_store_path = state_dir.join("ic_registry_local_store");

        let provisional_whitelist = Some(ProvisionalWhitelist::All);

        //     match self.provisional_whitelist.unwrap_or_default().as_str() {
        //     "*" => Some(ProvisionalWhitelist::All),
        //     "" => None,
        //     _ => {
        //         return Err(io::Error::new(
        //             io::ErrorKind::InvalidInput,
        //             "Whitelist can only be '*' or ''".to_string(),
        //         ))
        //     }
        // };

        let unit_delay = self.unit_delay_millis.map(Duration::from_millis);
        let initial_notary_delay = self.initial_notary_delay_millis.map(Duration::from_millis);

        let subnet_type = match self.subnet_type.as_deref() {
            Some("application") => SubnetType::Application,
            Some("verified_application") => SubnetType::VerifiedApplication,
            Some("system") | None => SubnetType::System,
            Some(s) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid subnet_type: {}", s),
                ))
            }
        };

        let ecdsa_keyid = self
            .ecdsa_keyid
            .as_ref()
            .map(|s| EcdsaKeyId::from_str(format!("{}-{}", s, node_index).as_str()))
            .transpose()
            .map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid ecdsa_keyid: {}", err),
                )
            })?;

        let mut chain_key_ids = self
            .chain_key_ids
            .iter()
            .map(|s| MasterPublicKeyId::from_str(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid chain_key_ids: {}", err),
                )
            })?;
        if let Some(ecdsa_key_id) = ecdsa_keyid {
            chain_key_ids.push(MasterPublicKeyId::Ecdsa(ecdsa_key_id));
        }

        Ok(ValidatedConfig {
            replica_version,
            log_level,
            state_dir,
            http_listen_addr,
            http_port_file,
            // metrics_addr,
            provisional_whitelist,
            artifact_pool_dir,
            crypto_root,
            state_manager_root,
            registry_local_store_path,
            unit_delay,
            initial_notary_delay,
            dkg_interval_length: self.dkg_interval_length.map(Height::from),
            consensus_pool_backend: self.consensus_pool_backend,
            subnet_features: to_subnet_features(&self.subnet_features),
            chain_key_ids,
            subnet_type,
            bitcoin_testnet_uds_path: self.bitcoin_testnet_uds_path,
            https_outcalls_uds_path: self.canister_http_uds_path,
            use_specified_ids_allocation_range: self.use_specified_ids_allocation_range,
        })
    }
}

fn to_subnet_features(features: &[String]) -> SubnetFeatures {
    let canister_sandboxing = features.iter().any(|s| s.as_str() == "canister_sandboxing");
    let http_requests = features.iter().any(|s| s.as_str() == "http_requests");
    SubnetFeatures {
        canister_sandboxing,
        http_requests,
        ..Default::default()
    }
}

#[derive(Debug)]
struct ValidatedConfig {
    replica_version: ReplicaVersion,
    log_level: ic_config::logger::Level,
    state_dir: PathBuf,
    http_listen_addr: SocketAddr,
    http_port_file: Option<PathBuf>,
    provisional_whitelist: Option<ProvisionalWhitelist>,
    artifact_pool_dir: PathBuf,
    crypto_root: PathBuf,
    state_manager_root: PathBuf,
    registry_local_store_path: PathBuf,
    unit_delay: Option<Duration>,
    initial_notary_delay: Option<Duration>,
    dkg_interval_length: Option<Height>,
    consensus_pool_backend: Option<String>,
    subnet_features: SubnetFeatures,
    chain_key_ids: Vec<MasterPublicKeyId>,
    subnet_type: SubnetType,
    bitcoin_testnet_uds_path: Option<PathBuf>,
    https_outcalls_uds_path: Option<PathBuf>,
    use_specified_ids_allocation_range: bool,
}

impl ValidatedConfig {
    fn build_replica_config(self: &ValidatedConfig) -> ReplicaConfig {
        let state_manager = Some(StateManagerConfig::new(self.state_manager_root.clone()));
        let http_handler = Some(HttpHandlerConfig {
            listen_addr: self.http_listen_addr,
            port_file_path: self.http_port_file.clone(),
            ..Default::default()
        });

        let mut artifact_pool_cfg =
            ArtifactPoolTomlConfig::new(self.artifact_pool_dir.clone(), None);
        // artifact_pool.rs picks "lmdb" if None here
        artifact_pool_cfg
            .consensus_pool_backend
            .clone_from(&self.consensus_pool_backend);
        let artifact_pool = Some(artifact_pool_cfg);

        let crypto = Some(CryptoConfig::new(self.crypto_root.clone()));
        let registry_client = Some(RegistryClientConfig {
            local_store: self.registry_local_store_path.clone(),
        });
        let logger_config = LoggerConfig {
            level: ic_config::logger::Level::Info,
            ..LoggerConfig::default()
        };
        let logger = Some(logger_config);

        let transport = Some(TransportConfig {
            node_ip: "0.0.0.0".to_string(),
            listening_port: 0,
            send_queue_size: 1024,
            ..Default::default()
        });

        let hypervisor = Some(hypervisor_config(self.subnet_features.canister_sandboxing));

        let adapters_config = Some(AdaptersConfig {
            bitcoin_testnet_uds_path: self.bitcoin_testnet_uds_path.clone(),
            https_outcalls_uds_path: self.https_outcalls_uds_path.clone(),
            ..AdaptersConfig::default()
        });

        ReplicaConfig {
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
        }
    }
}

pub fn hypervisor_config(canister_sandboxing: bool) -> HypervisorConfig {
    HypervisorConfig {
        canister_sandboxing_flag: if canister_sandboxing {
            FlagStatus::Enabled
        } else {
            FlagStatus::Disabled
        },
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
    }
}
