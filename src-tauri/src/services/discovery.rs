use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tauri::Emitter;

const SERVICE_TYPE: &str = "_dropmesh._udp.local.";

/// A device discovered on the local network (may or may not be paired)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub name: String,
    pub device_type: String,
    pub group_id_hash: String,
    pub host: String,
    pub port: u16,
    pub is_paired: bool,
}

/// Manages mDNS advertisement and discovery of other DropMesh devices
pub struct DiscoveryService {
    daemon: Option<ServiceDaemon>,
    /// Every peer currently visible on the network, keyed by device_id.
    /// The mDNS listener thread writes here; the rest of the app reads it to
    /// answer "is this paired device reachable right now, and at what address?"
    peers: Arc<RwLock<HashMap<String, DiscoveredPeer>>>,
}

impl DiscoveryService {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self {
            daemon: None,
            peers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Start advertising this device and listening for others.
    /// Emits `on_device_discovered` and `on_device_lost` Tauri events.
    pub fn start(
        &mut self,
        app_handle: tauri::AppHandle,
        device_id: &str,
        device_name: &str,
        device_type: &str,
        group_id_hash: &str,
        trusted_device_ids: Vec<String>,
        port: u16,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let daemon = ServiceDaemon::new()
            .map_err(|e| format!("Failed to create mDNS daemon: {}", e))?;

        // Build TXT record properties
        let mut properties = HashMap::new();
        properties.insert("v".to_string(), "1".to_string());
        properties.insert("id".to_string(), device_id.to_string());
        properties.insert("name".to_string(), device_name.chars().take(32).collect());
        properties.insert("type".to_string(), device_type.to_string());
        properties.insert("gid".to_string(), group_id_hash.to_string());

        // Get local IP
        let local_ip = get_local_ip();
        let hostname = get_hostname();

        log::info!(
            "Starting mDNS — device: {} | IP: {} | port: {}",
            device_name, local_ip, port
        );

        // Build instance name — must be unique per device
        let instance_name = format!("{}", &device_id[..8]);

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &hostname,
            local_ip,
            port,
            properties,
        )
        .map_err(|e| format!("Failed to build service info: {}", e))?;

        daemon
            .register(service_info)
            .map_err(|e| format!("Failed to register mDNS service: {}", e))?;

        log::info!("mDNS advertising started as '{}'", device_name);

        // Start browsing in a background thread (mDNS receiver is blocking)
        let receiver = daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| format!("Failed to start mDNS browse: {}", e))?;

        let my_device_id = device_id.to_string();
        let trusted_ids: HashSet<String> = trusted_device_ids.into_iter().collect();
        let peers = self.peers.clone();

        std::thread::spawn(move || {
            log::info!("mDNS listener thread started");

            loop {
                match receiver.recv() {
                    Ok(event) => match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let props = info.get_properties();

                            // Extract TXT record fields
                            let peer_id = props
                                .get("id")
                                .map(|v| v.val_str().to_string())
                                .unwrap_or_default();

                            // Don't announce ourselves
                            if peer_id == my_device_id || peer_id.is_empty() {
                                continue;
                            }

                            let peer_name = props
                                .get("name")
                                .map(|v| v.val_str().to_string())
                                .unwrap_or_else(|| "Unknown".to_string());

                            let peer_type = props
                                .get("type")
                                .map(|v| v.val_str().to_string())
                                .unwrap_or_else(|| "laptop".to_string());

                            let peer_gid = props
                                .get("gid")
                                .map(|v| v.val_str().to_string())
                                .unwrap_or_default();

                            let peer_port = info.get_port();
                            let peer_host = info
                                .get_addresses()
                                .iter()
                                .next()
                                .map(|a| a.to_string())
                                .unwrap_or_else(|| info.get_hostname().to_string());

                            let is_paired = trusted_ids.contains(&peer_id);

                            let peer = DiscoveredPeer {
                                device_id: peer_id.clone(),
                                name: peer_name.clone(),
                                device_type: peer_type,
                                group_id_hash: peer_gid,
                                host: peer_host,
                                port: peer_port,
                                is_paired,
                            };

                            log::info!(
                                "Discovered: {} ({}) at {}:{}",
                                peer_name, peer_id, peer.host, peer.port
                            );

                            if let Ok(mut map) = peers.write() {
                                map.insert(peer_id.clone(), peer.clone());
                            }

                            let _ = app_handle.emit("on_device_discovered", &peer);
                        }

                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            // Extract device ID from instance name
                            let instance = fullname
                                .trim_end_matches(&format!(".{}", SERVICE_TYPE))
                                .trim_end_matches('.');

                            log::info!("Device left network: {}", instance);

                            if let Ok(mut map) = peers.write() {
                                map.retain(|id, _| !id.starts_with(instance));
                            }

                            let _ = app_handle.emit("on_device_lost", &serde_json::json!({
                                "instance": instance
                            }));
                        }

                        ServiceEvent::SearchStarted(_) => {
                            log::debug!("mDNS search started");
                        }

                        _ => {}
                    },
                    Err(e) => {
                        log::error!("mDNS receiver error: {}", e);
                        break;
                    }
                }
            }

            log::warn!("mDNS listener thread exited");
        });

        self.daemon = Some(daemon);
        Ok(())
    }

    /// Snapshot of every peer currently visible on the network.
    pub fn visible_peers(&self) -> Vec<DiscoveredPeer> {
        self.peers.read().map(|m| m.values().cloned().collect()).unwrap_or_default()
    }

    /// Look up one peer by device_id, if it is currently visible.
    pub fn find_peer(&self, device_id: &str) -> Option<DiscoveredPeer> {
        self.peers.read().ok()?.get(device_id).cloned()
    }

    pub fn stop(&mut self) {
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.shutdown();
            log::info!("mDNS service stopped");
        }
    }
}

impl Drop for DiscoveryService {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Get the primary non-loopback IPv4 address of this machine
pub fn get_local_ip() -> std::net::IpAddr {
    if_addrs::get_if_addrs()
        .ok()
        .and_then(|addrs| {
            addrs
                .into_iter()
                .filter(|a| !a.is_loopback())
                .find_map(|a| match a.addr {
                    if_addrs::IfAddr::V4(v4) => Some(std::net::IpAddr::V4(v4.ip)),
                    _ => None,
                })
        })
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
}

/// Get this machine's hostname in mDNS format (e.g. "my-mac.local.")
pub fn get_hostname() -> String {
    let raw = hostname::get()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    // Ensure it ends with .local.
    if raw.ends_with(".local.") {
        raw
    } else if raw.ends_with(".local") {
        format!("{}.", raw)
    } else {
        format!("{}.local.", raw)
    }
}