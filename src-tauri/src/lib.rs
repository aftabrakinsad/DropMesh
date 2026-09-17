mod commands;
mod db;
mod models;
mod services;

use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

pub struct AppState {
    pub db: Arc<Mutex<db::Database>>,
    pub discovery: Arc<Mutex<services::discovery::DiscoveryService>>,
    pub transfer: Arc<Mutex<services::transfer::TransferEngine>>,
    pub storage: Arc<Mutex<services::storage::StorageManager>>,
    pub queue: Arc<Mutex<services::queue::QueueService>>,
    pub security: Arc<services::security::SecurityModule>,
    pub delivery: Arc<services::delivery_watcher::DeliveryWatcher>,
}

pub fn run() {
    // rustls needs a crypto provider registered before any TLS/QUIC use
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_handle = app.handle().clone();

            // ── Database ──────────────────────────────────────────────────────
            let db = db::Database::new(&app_handle)
                .expect("Failed to initialize database");
            db.run_migrations()
                .expect("Failed to run database migrations");

            // ── Services ──────────────────────────────────────────────────────
            let security = services::security::SecurityModule::new(&db)
                .expect("Failed to initialize security module");
            let storage = services::storage::StorageManager::new(&db)
                .expect("Failed to initialize storage manager");
            let transfer = services::transfer::TransferEngine::new()
                .expect("Failed to initialize transfer engine");
            let staging_path = std::path::PathBuf::from(&storage.get_stats().folder_path)
                .join("staging");
            let queue = services::queue::QueueService::new(staging_path);
            let discovery = services::discovery::DiscoveryService::new()
                .expect("Failed to initialize discovery service");

            let db_arc = Arc::new(Mutex::new(db));
            let transfer_arc = Arc::new(Mutex::new(transfer));
            let queue_arc = Arc::new(Mutex::new(queue));
            let storage_arc = Arc::new(Mutex::new(storage));

            let delivery = services::delivery_watcher::DeliveryWatcher::new(
                db_arc.clone(),
                queue_arc.clone(),
                transfer_arc.clone(),
            );
            let delivery_arc = Arc::new(delivery);

            // ── Shared state ──────────────────────────────────────────────────
            let state = AppState {
                db: db_arc.clone(),
                discovery: Arc::new(Mutex::new(discovery)),
                transfer: transfer_arc.clone(),
                storage: storage_arc.clone(),
                queue: queue_arc.clone(),
                security: Arc::new(security),
                delivery: delivery_arc.clone(),
            };
            app.manage(state);

            // ── Start QUIC listener, then advertise its real port ────────────
            //
            // Order matters: mDNS must advertise the port the listener actually
            // bound to. Binding a fixed port breaks when two instances share a
            // machine, so the OS assigns one and we publish that.
            let transfer_for_listener = transfer_arc.clone();
            let db_for_listener = db_arc.clone();
            let storage_for_listener = storage_arc.clone();
            let discovery_arc = app.state::<AppState>().inner().discovery.clone();
            let app_handle_bg = app_handle.clone();

            tauri::async_runtime::spawn(async move {
                // 1. Bind the QUIC listener
                let port = {
                    let mut engine = transfer_for_listener.lock().await;
                    match engine
                        .start_listener(
                            app_handle_bg.clone(),
                            db_for_listener.clone(),
                            storage_for_listener,
                        )
                        .await
                    {
                        Ok(p) => p,
                        Err(e) => {
                            log::error!("QUIC listener failed to start: {}", e);
                            return;
                        }
                    }
                };

                // 2. Gather identity for the mDNS advertisement
                let (device_id, device_name, device_type, group_hash, trusted_ids) = {
                    let db = db_for_listener.lock().await;

                    let row = db.conn().query_row(
                        "SELECT id, name, type FROM device_self LIMIT 1",
                        [],
                        |r| Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        )),
                    );

                    let (id, name, dtype) = match row {
                        Ok(v) => v,
                        Err(e) => {
                            log::error!("No device identity for discovery: {}", e);
                            return;
                        }
                    };

                    let group_hash = db.conn()
                        .query_row(
                            "SELECT group_id FROM trust_group LIMIT 1",
                            [],
                            |r| r.get::<_, String>(0),
                        )
                        .map(|g| services::security::SecurityModule::group_id_hash(&g))
                        .unwrap_or_else(|_| "00000000".to_string());

                    let trusted: Vec<String> = db.conn()
                        .prepare("SELECT device_id FROM paired_devices")
                        .ok()
                        .and_then(|mut stmt| {
                            stmt.query_map([], |r| r.get(0))
                                .ok()
                                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                        })
                        .unwrap_or_default();

                    (id, name, dtype, group_hash, trusted)
                };

                // 3. Advertise on mDNS with the real listener port
                let mut discovery = discovery_arc.lock().await;
                match discovery.start(
                    app_handle_bg,
                    &device_id,
                    &device_name,
                    &device_type,
                    &group_hash,
                    trusted_ids,
                    port,
                ) {
                    Ok(_) => log::info!(
                        "Advertising '{}' on mDNS, transfer port {}",
                        device_name, port
                    ),
                    Err(e) => log::error!("Failed to start discovery: {}", e),
                }
            });

            // ── Start delivery watcher ────────────────────────────────────────
            app.state::<AppState>().inner().delivery.start(app_handle.clone());

            // ── Delivery poller ───────────────────────────────────────────────
            //
            // mDNS only announces a service once, so a discovery event can't be
            // relied on to trigger delivery — a device paired after it was
            // discovered would never get its queue flushed. Instead, poll the
            // live peer registry and push to any paired device that is both
            // visible and has work waiting.
            let delivery_poll = delivery_arc.clone();
            let discovery_poll = app.state::<AppState>().inner().discovery.clone();
            let db_poll = db_arc.clone();

            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                    // Peers currently visible on the network
                    let peers = {
                        let d = discovery_poll.lock().await;
                        d.visible_peers()
                    };
                    if peers.is_empty() {
                        continue;
                    }

                    // Paired device ids that have at least one pending file
                    let targets: Vec<String> = {
                        let db = db_poll.lock().await;
                        db.conn()
                            .prepare(
                                "SELECT DISTINCT q.target_device_id
                                 FROM file_queue q
                                 INNER JOIN paired_devices p
                                     ON p.device_id = q.target_device_id
                                 WHERE q.status = 'pending'",
                            )
                            .ok()
                            .and_then(|mut stmt| {
                                stmt.query_map([], |r| r.get::<_, String>(0))
                                    .ok()
                                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
                            })
                            .unwrap_or_default()
                    };

                    for device_id in targets {
                        if let Some(peer) = peers.iter().find(|p| p.device_id == device_id) {
                            delivery_poll
                                .on_device_online(&device_id, &peer.host, peer.port)
                                .await;
                        }
                    }
                }
            });

            log::info!("DropMesh initialized");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::device::get_self_device,
            commands::device::update_device_name,
            commands::pairing::start_pairing,
            commands::pairing::pair_with_device,
            commands::pairing::get_paired_devices,
            commands::pairing::remove_device,
            commands::pairing::clear_all_devices,
            commands::transfer::send_files,
            commands::transfer::cancel_transfer,
            commands::transfer::get_transfers,
            commands::storage::get_storage_stats,
            commands::storage::update_storage_config,
            commands::storage::set_storage_folder,
            commands::queue::queue_file,
            commands::queue::get_queue,
            commands::queue::get_queue_stats,
            commands::queue::cancel_queued_file,
            commands::queue::retry_queued_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DropMesh");
}