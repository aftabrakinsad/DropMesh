use crate::models::transfer::StorageStats;
use crate::AppState;
use tauri::State;

/// Get current storage usage and configuration
#[tauri::command]
pub async fn get_storage_stats(state: State<'_, AppState>) -> Result<StorageStats, String> {
    let storage = state.storage.lock().await;
    Ok(storage.get_stats())
}

/// Update storage quota and file size cap
#[tauri::command]
pub async fn update_storage_config(
    state: State<'_, AppState>,
    quota_bytes: Option<u64>,
    file_size_cap_bytes: Option<u64>,
) -> Result<StorageStats, String> {
    let db = state.db.lock().await;
    let mut storage = state.storage.lock().await;

    if let Some(quota) = quota_bytes {
        if quota < 104_857_600 {
            return Err("Minimum quota is 100 MB".to_string());
        }
        storage
            .set_quota(&db, quota)
            .map_err(|e| format!("Failed to update quota: {}", e))?;
    }

    if let Some(cap) = file_size_cap_bytes {
        if cap < 104_857_600 {
            return Err("Minimum file size cap is 100 MB".to_string());
        }
        storage
            .set_file_size_cap(&db, cap)
            .map_err(|e| format!("Failed to update file size cap: {}", e))?;
    }

    Ok(storage.get_stats())
}

/// Set the storage folder path
#[tauri::command]
pub async fn set_storage_folder(
    state: State<'_, AppState>,
    folder_path: String,
) -> Result<StorageStats, String> {
    let path = std::path::PathBuf::from(&folder_path);

    if !path.exists() {
        std::fs::create_dir_all(&path)
            .map_err(|e| format!("Cannot create folder: {}", e))?;
    }

    let db = state.db.lock().await;
    let mut storage = state.storage.lock().await;

    storage
        .set_folder_path(&db, path)
        .map_err(|e| format!("Failed to set folder: {}", e))?;

    Ok(storage.get_stats())
}
