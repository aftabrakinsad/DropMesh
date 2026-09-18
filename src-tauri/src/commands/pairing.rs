use crate::models::device::{PairedDevice, PairingPayload};
use crate::services::pairing_handshake::PairMessage;
use crate::services::security::SecurityModule;
use crate::AppState;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::State;

/// DEVICE A: Generate a pairing PIN and wait for Device B to connect.
/// The PIN is displayed in the UI. Device A's QUIC listener will handle
/// the incoming PairRequest automatically.
#[tauri::command]
pub async fn start_pairing(state: State<'_, AppState>) -> Result<PairingPayload, String> {
    let db = state.db.lock().await;

    let (device_id, public_key): (String, Vec<u8>) = db
        .conn()
        .query_row(
            "SELECT id, public_key FROM device_self LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("No device identity: {}", e))?;

    let group_id = db
        .conn()
        .query_row("SELECT group_id FROM trust_group LIMIT 1", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());

    // Generate 6-digit PIN
    let code: u32 = rand::random::<u32>() % 900_000 + 100_000;
    let pairing_code = format!("{:06}", code);

    let expires = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::minutes(5))
        .unwrap()
        .to_rfc3339();

    // Derive the group key from the PIN and store it.
    // Device B will derive the identical key from the same PIN.
    let group_key = SecurityModule::derive_group_key_from_pairing_code(&pairing_code)
        .map_err(|e| format!("Key derivation failed: {}", e))?;

    db.conn()
        .execute(
            "INSERT OR REPLACE INTO trust_group (group_id, group_key_encrypted) VALUES (?1, ?2)",
            rusqlite::params![group_id, group_key],
        )
        .map_err(|e| format!("Failed to store group key: {}", e))?;

    // Store the active PIN so the QUIC handler can validate incoming PairRequests
    let _ = db.conn().execute(
        "INSERT OR REPLACE INTO storage_config (key, value) VALUES ('active_pairing_pin', ?1)",
        [&pairing_code],
    );
    let _ = db.conn().execute(
        "INSERT OR REPLACE INTO storage_config (key, value) VALUES ('active_pairing_expires', ?1)",
        [&expires],
    );
    let _ = db.conn().execute(
        "INSERT OR REPLACE INTO storage_config (key, value) VALUES ('active_pairing_group', ?1)",
        [&group_id],
    );

    let verification_emojis = SecurityModule::derive_verification_emojis(&group_key).join(" ");

    log::info!(
        "Pairing ready — PIN: {} | waiting for peer to connect",
        pairing_code
    );

    Ok(PairingPayload {
        device_id,
        public_key: STANDARD.encode(&public_key),
        group_id,
        nonce: uuid::Uuid::new_v4().to_string(),
        expires,
        pairing_code,
        verification_emojis,
    })
}

/// DEVICE B: Connect to Device A and perform the real pairing handshake.
///
/// `peer_host` and `peer_port` come from mDNS discovery — the user taps
/// a device in the "Nearby" list, so we know its real network address.
#[tauri::command]
pub async fn pair_with_device(
    state: State<'_, AppState>,
    peer_host: String,
    peer_port: u16,
    expected_peer_device_id: String,
    pin: String,
) -> Result<serde_json::Value, String> {
    let pin = pin.replace(' ', "");
    if pin.len() != 6 || !pin.chars().all(|c| c.is_ascii_digit()) {
        return Err("Enter a valid 6-digit code".to_string());
    }
    if expected_peer_device_id.trim().is_empty() {
        return Err("Missing target device identity".to_string());
    }

    // Gather our own identity to send to Device A
    let (my_id, my_name, my_type, my_pubkey): (String, String, String, Vec<u8>) = {
        let db = state.db.lock().await;
        db.conn()
            .query_row(
                "SELECT id, name, type, public_key FROM device_self LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|e| format!("No device identity: {}", e))?
    };

    let my_cert_fingerprint: String = {
        let db = state.db.lock().await;
        db.conn()
            .query_row(
                "SELECT value FROM storage_config WHERE key = 'local_cert_fingerprint'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_default()
    };

    if my_cert_fingerprint.is_empty() {
        return Err("Secure channel is not ready yet. Please try again.".to_string());
    }

    let request = PairMessage::PairRequest {
        device_id: my_id.clone(),
        name: my_name,
        device_type: my_type,
        public_key: STANDARD.encode(&my_pubkey),
        cert_fingerprint: my_cert_fingerprint,
        pin: pin.clone(),
    };

    // Connect over QUIC and exchange identities
    let (response, observed_cert_fp) =
        crate::services::transfer::send_pair_request(&peer_host, peer_port, request)
        .await
        .map_err(|e| format!("Could not reach the other device: {}", e))?;

    match response {
        PairMessage::PairResponse {
            device_id,
            name,
            device_type,
            public_key,
            cert_fingerprint,
            group_id,
            verification_emojis,
        } => {
            if device_id != expected_peer_device_id {
                return Err("Device identity mismatch. Pairing was blocked.".to_string());
            }
            if !cert_fingerprint.is_empty() && cert_fingerprint != observed_cert_fp {
                return Err("Certificate mismatch detected. Pairing was blocked.".to_string());
            }

            // Derive the same group key from the PIN
            let group_key = SecurityModule::derive_group_key_from_pairing_code(&pin)
                .map_err(|e| format!("Key derivation failed: {}", e))?;

            let peer_pubkey_bytes = STANDARD
                .decode(&public_key)
                .map_err(|e| format!("Invalid peer public key: {}", e))?;

            // Store Device A with its REAL identity
            let db = state.db.lock().await;
            db.conn()
                .execute(
                    "INSERT OR REPLACE INTO trust_group (group_id, group_key_encrypted) VALUES (?1, ?2)",
                    rusqlite::params![group_id, group_key],
                )
                .map_err(|e| format!("Failed to store group key: {}", e))?;

            db.conn()
                .execute(
                    "INSERT OR REPLACE INTO paired_devices
                     (device_id, name, type, public_key, group_id, last_seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
                    rusqlite::params![&device_id, &name, &device_type, peer_pubkey_bytes, &group_id],
                )
                .map_err(|e| format!("Failed to store peer: {}", e))?;

            let key = format!("peer_cert_fp:{}", device_id);
            db.conn()
                .execute(
                    "INSERT OR REPLACE INTO storage_config (key, value) VALUES (?1, ?2)",
                    rusqlite::params![key, observed_cert_fp],
                )
                .map_err(|e| format!("Failed to store peer certificate fingerprint: {}", e))?;

            log::info!("Paired with {} ({}) — emojis: {}", name, device_id, verification_emojis);

            Ok(serde_json::json!({
                "success": true,
                "peer_device_id": device_id,
                "peer_name": name,
                "verification_emojis": verification_emojis,
                "group_id": group_id,
            }))
        }
        PairMessage::PairRejected { reason } => Err(reason),
        _ => Err("Unexpected response from the other device".to_string()),
    }
}

/// List paired devices, marking each online if mDNS currently sees it.
#[tauri::command]
pub async fn get_paired_devices(
    state: State<'_, AppState>,
) -> Result<Vec<PairedDevice>, String> {
    // Which device_ids are visible on the network right now
    let visible: std::collections::HashSet<String> = {
        let discovery = state.discovery.lock().await;
        discovery.visible_peers().into_iter().map(|p| p.device_id).collect()
    };

    let db = state.db.lock().await;

    let mut stmt = db
        .conn()
        .prepare(
            "SELECT device_id, name, type, public_key, group_id, paired_at, last_seen_at
             FROM paired_devices ORDER BY name",
        )
        .map_err(|e| format!("Query failed: {}", e))?;

    let devices = stmt
        .query_map([], |row| {
            let pubkey_bytes: Vec<u8> = row.get(3)?;
            let id: String = row.get(0)?;
            Ok(PairedDevice {
                device_id: id.clone(),
                name: row.get(1)?,
                device_type: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?))
                    .unwrap_or(crate::models::device::DeviceType::Laptop),
                public_key: STANDARD.encode(&pubkey_bytes),
                group_id: row.get(4)?,
                paired_at: row.get(5)?,
                last_seen_at: row.get(6)?,
                is_online: visible.contains(&id),
            })
        })
        .map_err(|e| format!("Failed to read devices: {}", e))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(devices)
}

/// Remove a device and rotate the group key so it can't decrypt future transfers.
#[tauri::command]
pub async fn remove_device(state: State<'_, AppState>, device_id: String) -> Result<(), String> {
    let db = state.db.lock().await;

    let removed = db
        .conn()
        .execute("DELETE FROM paired_devices WHERE device_id = ?1", [&device_id])
        .map_err(|e| format!("Failed to remove device: {}", e))?;

    if removed == 0 {
        return Err("Device not found".to_string());
    }

    // Rotate the group key — the removed device must not be able to decrypt
    // anything sent after its removal.
    if let Ok((group_id, old_key)) = db.conn().query_row(
        "SELECT group_id, group_key_encrypted FROM trust_group LIMIT 1",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
    ) {
        let rotation_nonce = uuid::Uuid::new_v4().to_string();
        if let Ok(new_key) = SecurityModule::rotate_group_key(&old_key, &rotation_nonce) {
            let _ = db.conn().execute(
                "UPDATE trust_group SET group_key_encrypted = ?1 WHERE group_id = ?2",
                rusqlite::params![new_key, group_id],
            );
            log::info!("Group key rotated after removing {}", device_id);
        }
    }

    log::info!("Removed device: {}", device_id);
    Ok(())
}

/// Remove every paired device — useful for clearing stale entries.
#[tauri::command]
pub async fn clear_all_devices(state: State<'_, AppState>) -> Result<u32, String> {
    let db = state.db.lock().await;
    let removed = db
        .conn()
        .execute("DELETE FROM paired_devices", [])
        .map_err(|e| format!("Failed to clear devices: {}", e))?;
    log::info!("Cleared {} paired device(s)", removed);
    Ok(removed as u32)
}