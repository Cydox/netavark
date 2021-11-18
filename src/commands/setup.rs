//! Configures the given network namespace with provided specs
use crate::error::NetavarkError;
use crate::firewall;
use crate::firewall::iptables::MAX_HASH_SIZE;
use crate::network;
use crate::network::core_utils::CoreUtils;
use crate::network::{core_utils, types};
use clap::{self, Clap};
use log::debug;
use std::collections::HashMap;
use std::error::Error;

const IPV4_FORWARD: &str = "net.ipv4.ip_forward";

#[derive(Clap, Debug)]
pub struct Setup {
    /// Network namespace path
    #[clap(forbid_empty_values = true, required = true)]
    network_namespace_path: String,
}

impl Setup {
    /// The setup command configures the given network namespace with the given configuration, creating any interfaces and firewall rules necessary.
    pub fn new(network_namespace_path: String) -> Self {
        Self {
            network_namespace_path,
        }
    }

    pub fn exec(&self, input_file: String) -> Result<(), Box<dyn Error>> {
        match network::validation::ns_checks(&self.network_namespace_path) {
            Ok(_) => (),
            Err(e) => {
                bail!("invalid namespace path: {}", e);
            }
        }
        debug!("{:?}", "Setting up...");

        let network_options = match network::types::NetworkOptions::load(&input_file) {
            Ok(opts) => opts,
            Err(e) => {
                return Err(Box::new(NetavarkError {
                    error: format!("{}", e),
                    errno: 1,
                }));
            }
        };

        let firewall_driver = match firewall::get_supported_firewall_driver() {
            Ok(driver) => driver,
            Err(e) => panic!("{}", e.to_string()),
        };

        // Sysctl setup
        // set ip forwarding to 1
        core_utils::CoreUtils::apply_sysctl_value(IPV4_FORWARD, "1")?;

        let mut response: HashMap<String, types::StatusBlock> = HashMap::new();

        // Perform per-network setup
        for (net_name, network) in network_options.network_info.iter() {
            debug!(
                "Setting up network {} with driver {}",
                net_name, network.driver
            );

            match network.driver.as_str() {
                "bridge" => {
                    let per_network_opts =
                        network_options.networks.get(net_name).ok_or_else(|| {
                            std::io::Error::new(
                                std::io::ErrorKind::Other,
                                format!("network options for network {} not found", net_name),
                            )
                        })?;
                    //Configure Bridge and veth_pairs
                    let status_block = network::core::Core::bridge_per_podman_network(
                        per_network_opts,
                        network,
                        &self.network_namespace_path,
                    )?;
                    response.insert(net_name.to_owned(), status_block);

                    let id_network_hash = CoreUtils::create_network_hash(net_name, MAX_HASH_SIZE);
                    firewall_driver.setup_network(network.clone(), id_network_hash.clone())?;

                    let port_bindings = network_options.port_mappings.clone();
                    match port_bindings {
                        None => {}
                        Some(i) => {
                            firewall_driver.setup_port_forward(
                                network.clone(),
                                &network_options.container_id,
                                i,
                                net_name,
                                &id_network_hash,
                                per_network_opts,
                                // &id_network_hash.as_str()[0..MAX_HASH_SIZE],
                            )?;
                        }
                    }
                }
                // unknown driver
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("unknown network driver {}", network.driver),
                    )
                    .into());
                }
            }
        }

        debug!("{:#?}", response);
        let response_json = serde_json::to_string(&response)?;
        println!("{}", response_json);
        debug!("{:?}", "Setup complete");
        Ok(())
    }
}
