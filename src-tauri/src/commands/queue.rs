use crate::models::queue::{QueueEntry, QueueStats};
use crate::AppState;
use tauri::State;

/// Queue a file for async delivery to a target device.
/// The file is staged locally and will be sent automatically
/// when the target device comes online.
#[tauri::command]
pub async fn queue_file(
    state: State<'_, AppState>,
    file_path: String,
    target_device_id: String,
) -> Result<String, String> {
    // Validate the file exists
    let path = std::path::PathBuf::from(&file_path);
    if !path.exists() {
        return Err(format!("File not found: {}", file_path));
    }

    let metadata = std::fs::metadata(&path)
        .map_err(|e| format!("Cannot read file: {}", e))?;

    // Check storage quota
    {
        let storage = state.storage.lock().await;
        if !storage.can_accept_file(metadata.len()) {
            return Err("Not enough storage quota to stage this file".to_string());
        }
    }

    // Hash the file
    let file_bytes = std::fs::read(&path)
        .map_err(|e| format!("Cannot read file: {}", e))?;
    let sha256 = crate::services::security::SecurityModule::hash_file(&file_bytes);

    let file_name = path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Enqueue
    let db = state.db.lock().await;
    let queue = state.queue.lock().await;
    let queue_id = queue
        .enqueue(
            db.conn(),
            &path,
            &target_device_id,
            &file_name,
            metadata.len(),
            &sha256,
        )
        .map_err(|e| format!("Failed to queue file: {}", e))?;

    Ok(queue_id)
}

/// Get all queue entries for the UI.
#[tauri::command]
pub async fn get_queue(state: State<'_, AppState>) -> Result<Vec<QueueEntry>, String> {
    let db = state.db.lock().await;
    let queue = state.queue.lock().await;
    queue
        .get_all_entries(db.conn())
        .map_err(|e| format!("Failed to get queue: {}", e))
}

/// Get queue statistics.
#[tauri::command]
pub async fn get_queue_stats(state: State<'_, AppState>) -> Result<QueueStats, String> {
    let db = state.db.lock().await;
    let queue = state.queue.lock().await;
    queue
        .get_stats(db.conn())
        .map_err(|e| format!("Failed to get queue stats: {}", e))
}

/// Cancel a pending queue entry.
#[tauri::command]
pub async fn cancel_queued_file(
    state: State<'_, AppState>,
    queue_id: String,
) -> Result<(), String> {
    let db = state.db.lock().await;
    let queue = state.queue.lock().await;
    queue
        .cancel(db.conn(), &queue_id)
        .map_err(|e| format!("Failed to cancel: {}", e))
}

/// Retry a failed queue entry by resetting it to pending.
#[tauri::command]
pub async fn retry_queued_file(
    state: State<'_, AppState>,
    queue_id: String,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.conn()
        .execute(
            "UPDATE file_queue SET status = 'pending', error_message = NULL WHERE queue_id = ?1 AND status = 'failed'",
            [&queue_id],
        )
        .map_err(|e| format!("Failed to retry: {}", e))?;
    Ok(())
}
