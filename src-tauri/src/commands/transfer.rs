use crate::models::transfer::Transfer;
use crate::AppState;
use tauri::State;

/// Initiate file transfer to a paired device
#[tauri::command]
pub async fn send_files(
    state: State<'_, AppState>,
    file_paths: Vec<String>,
    target_device_id: String,
) -> Result<Vec<String>, String> {
    // Validate target device is paired and load group key
    let (group_key, peer_cert_fingerprint, sender_device_id): (Vec<u8>, String, String) = {
        let db = state.db.lock().await;
        let key = db
            .conn()
            .query_row(
                "SELECT tg.group_key_encrypted
                 FROM trust_group tg
                 INNER JOIN paired_devices pd ON pd.group_id = tg.group_id
                 WHERE pd.device_id = ?1",
                [&target_device_id],
                |row| row.get(0),
            )
            .map_err(|_| "Target device is not paired or has no group key".to_string())?;

        let cert_key = format!("peer_cert_fp:{}", target_device_id);
        let cert_fp = db
            .conn()
            .query_row(
                "SELECT value FROM storage_config WHERE key = ?1",
                [cert_key],
                |row| row.get(0),
            )
            .map_err(|_| "No pinned certificate for this device. Re-pair it to continue.".to_string())?;
        let sender_id = db
            .conn()
            .query_row(
                "SELECT id FROM device_self LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| "No local device identity found".to_string())?;
        (key, cert_fp, sender_id)
    };

    let peer = {
        let discovery = state.discovery.lock().await;
        discovery.find_peer(&target_device_id)
    }
    .ok_or_else(|| "Target device is offline or not discoverable".to_string())?;

    // Check storage for each file
    let storage = state.storage.lock().await;
    for path in &file_paths {
        let metadata = std::fs::metadata(path)
            .map_err(|e| format!("Cannot read file {}: {}", path, e))?;

        if !storage.can_accept_file(metadata.len()) {
            return Err(format!(
                "File '{}' exceeds storage limits",
                path
            ));
        }
    }
    drop(storage);

    // Queue each file for transfer
    let mut transfer_ids = Vec::new();

    for path in &file_paths {
        let result = {
            let mut engine = state.transfer.lock().await;
            engine
                .send_file(
                    std::path::PathBuf::from(path),
                    &peer.host,
                    peer.port,
                    &group_key,
                    Some(&peer_cert_fingerprint),
                    &sender_device_id,
                )
                .await
        };

        match result {
            Ok(tid) => {
                let db = state.db.lock().await;
                let file_name = std::path::Path::new(path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

                let _ = db.conn().execute(
                    "INSERT INTO transfers (transfer_id, direction, peer_device_id, file_name, file_size, sha256, status, chunks_total) VALUES (?1, 'sent', ?2, ?3, ?4, '', 'queued', 0)",
                    rusqlite::params![tid, target_device_id, file_name.to_string(), file_size as i64],
                );

                transfer_ids.push(tid);
            }
            Err(e) => {
                log::error!("Failed to queue file {}: {}", path, e);
            }
        }
    }

    Ok(transfer_ids)
}

/// Cancel an in-progress transfer
#[tauri::command]
pub async fn cancel_transfer(
    state: State<'_, AppState>,
    transfer_id: String,
) -> Result<(), String> {
    let mut engine = state.transfer.lock().await;
    engine
        .cancel_transfer(&transfer_id)
        .map_err(|e| format!("Failed to cancel: {}", e))?;

    let db = state.db.lock().await;
    let _ = db.conn().execute(
        "UPDATE transfers SET status = 'cancelled' WHERE transfer_id = ?1",
        [&transfer_id],
    );

    Ok(())
}

/// Get transfer history with optional status filter
#[tauri::command]
pub async fn get_transfers(
    state: State<'_, AppState>,
    status_filter: Option<String>,
) -> Result<Vec<Transfer>, String> {
    let db = state.db.lock().await;

    let base_select = "SELECT transfer_id, direction, peer_device_id, file_name, file_size, mime_type, sha256, status, chunks_total, chunks_completed, started_at, completed_at, error_message FROM transfers";

    let parse_transfer = |row: &rusqlite::Row<'_>| {
            Ok(Transfer {
                transfer_id: row.get(0)?,
                direction: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(1)?))
                    .unwrap_or(crate::models::transfer::TransferDirection::Sent),
                peer_device_id: row.get(2)?,
                peer_device_name: None,
                file_name: row.get(3)?,
                file_size: row.get::<_, i64>(4)? as u64,
                mime_type: row.get(5)?,
                sha256: row.get(6)?,
                status: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(7)?))
                    .unwrap_or(crate::models::transfer::TransferStatus::Failed),
                chunks_total: row.get::<_, i32>(8)? as u32,
                chunks_completed: row.get::<_, i32>(9)? as u32,
                started_at: row.get(10)?,
                completed_at: row.get(11)?,
                error_message: row.get(12)?,
            })
    };

    let transfers = if let Some(status) = status_filter {
        let query = format!("{base_select} WHERE status = ?1 ORDER BY started_at DESC");
        let mut stmt = db
            .conn()
            .prepare(&query)
            .map_err(|e| format!("Query failed: {}", e))?;
        stmt.query_map([status], parse_transfer)
            .map_err(|e| format!("Failed to read transfers: {}", e))?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        let query = format!("{base_select} ORDER BY started_at DESC");
        let mut stmt = db
            .conn()
            .prepare(&query)
            .map_err(|e| format!("Query failed: {}", e))?;
        stmt.query_map([], parse_transfer)
            .map_err(|e| format!("Failed to read transfers: {}", e))?
            .filter_map(|r| r.ok())
            .collect()
    };

    Ok(transfers)
}
