use crate::db::Database;
use crate::services::queue::QueueService;
use crate::services::transfer::TransferEngine;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::Mutex;

pub struct DeliveryWatcher {
    db: Arc<Mutex<Database>>,
    queue: Arc<Mutex<QueueService>>,
    transfer: Arc<Mutex<TransferEngine>>,
}

impl DeliveryWatcher {
    pub fn new(
        db: Arc<Mutex<Database>>,
        queue: Arc<Mutex<QueueService>>,
        transfer: Arc<Mutex<TransferEngine>>,
    ) -> Self {
        Self { db, queue, transfer }
    }

    pub fn start(&self, app_handle: tauri::AppHandle) {
        let db = self.db.clone();
        let queue = self.queue.clone();

        // Periodic maintenance: expire old entries, clean up delivered ones
        tauri::async_runtime::spawn(async move {
            log::info!("Delivery watcher started");
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                let db_guard = db.lock().await;
                let queue_guard = queue.lock().await;
                if let Err(e) = queue_guard.expire_old_entries(db_guard.conn()) {
                    log::error!("Failed to expire queue entries: {}", e);
                }
                if let Err(e) = queue_guard.cleanup_delivered(db_guard.conn()) {
                    log::error!("Failed to cleanup delivered entries: {}", e);
                }
            }
        });

        // Queue stats emission for frontend
        let db2 = self.db.clone();
        let queue2 = self.queue.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                let db_guard = db2.lock().await;
                let queue_guard = queue2.lock().await;
                if let Ok(stats) = queue_guard.get_stats(db_guard.conn()) {
                    if stats.pending_count > 0 || stats.delivering_count > 0 {
                        let _ = app_handle.emit("queue_stats_update", &stats);
                    }
                }
            }
        });
    }

    /// Called when a paired device comes online via mDNS.
    /// Finds all pending queue entries for that device and delivers them.
    pub async fn on_device_online(
        &self,
        device_id: &str,
        peer_host: &str,
        peer_port: u16,
    ) {
        let db = self.db.lock().await;

        // Look up group key for this device
        let group_key: Option<Vec<u8>> = db.conn().query_row(
            "SELECT tg.group_key_encrypted FROM trust_group tg
             INNER JOIN paired_devices pd ON pd.group_id = tg.group_id
             WHERE pd.device_id = ?1",
            [device_id],
            |row| row.get(0),
        ).ok();

        let group_key = match group_key {
            Some(k) => k,
            None => {
                log::warn!("Device {} online but no group key found", device_id);
                return;
            }
        };

        let queue = self.queue.lock().await;
        let pending = match queue.get_pending_for_device(db.conn(), device_id) {
            Ok(entries) => entries,
            Err(e) => {
                log::error!("Failed to check queue for {}: {}", device_id, e);
                return;
            }
        };

        if pending.is_empty() {
            log::info!("Device {} online — no queued files", device_id);
            return;
        }

        log::info!(
            "Device {} online — {} file(s) queued for delivery",
            device_id, pending.len()
        );

        // Get this device's ID for use as sender
        let sender_id: String = db.conn().query_row(
            "SELECT id FROM device_self LIMIT 1",
            [],
            |row| row.get(0),
        ).unwrap_or_default();

        drop(db);
        drop(queue);

        // Deliver each pending file
        for entry in &pending {
            let staged = std::path::PathBuf::from(&entry.staging_path);
            if !staged.exists() {
                log::error!("Staged file missing for queue entry {}", entry.queue_id);
                let db = self.db.lock().await;
                let queue = self.queue.lock().await;
                let _ = queue.mark_failed(db.conn(), &entry.queue_id, "Staged file not found");
                continue;
            }

            // Mark as delivering
            {
                let db = self.db.lock().await;
                let queue = self.queue.lock().await;
                if let Err(e) = queue.mark_delivering(db.conn(), &entry.queue_id) {
                    log::error!("Failed to mark delivering: {}", e);
                    continue;
                }
            }

            log::info!(
                "Delivering {} ({} bytes) to {}",
                entry.file_name, entry.file_size, device_id
            );

            let mut engine = self.transfer.lock().await;
            let result = engine.send_file(
                staged,
                peer_host,
                peer_port,
                &group_key,
                None, // TODO: pass pinned peer cert from pairing
                &sender_id,
            ).await;

            let db = self.db.lock().await;
            let queue = self.queue.lock().await;

            match result {
                Ok(_) => {
                    if let Err(e) = queue.mark_delivered(db.conn(), &entry.queue_id) {
                        log::error!("Failed to mark delivered: {}", e);
                    } else {
                        log::info!("Delivered: {}", entry.file_name);
                    }
                }
                Err(e) => {
                    log::error!("Delivery failed for {}: {}", entry.file_name, e);
                    let _ = queue.mark_failed(db.conn(), &entry.queue_id, &e.to_string());
                }
            }
        }
    }
}