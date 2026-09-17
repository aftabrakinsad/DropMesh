use crate::models::device::SelfDevice;
use crate::AppState;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::State;

/// Get this device's identity information
#[tauri::command]
pub async fn get_self_device(state: State<'_, AppState>) -> Result<SelfDevice, String> {
    let db = state.db.lock().await;

    db.conn()
        .query_row(
            "SELECT id, name, type, public_key, created_at FROM device_self LIMIT 1",
            [],
            |row| {
                let pubkey_bytes: Vec<u8> = row.get(3)?;
                Ok(SelfDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    device_type: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?))
                        .unwrap_or(crate::models::device::DeviceType::Laptop),
                    public_key: STANDARD.encode(&pubkey_bytes),
                    created_at: row.get(4)?,
                })
            },
        )
        .map_err(|e| format!("Failed to get device info: {}", e))
}

/// Update this device's display name
#[tauri::command]
pub async fn update_device_name(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    if name.is_empty() || name.len() > 32 {
        return Err("Name must be 1-32 characters".to_string());
    }

    let db = state.db.lock().await;

    db.conn()
        .execute("UPDATE device_self SET name = ?1", [&name])
        .map_err(|e| format!("Failed to update name: {}", e))?;

    log::info!("Device name updated to '{}'", name);
    Ok(())
}
