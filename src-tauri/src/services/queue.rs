use crate::models::queue::*;
use rusqlite::params;
use std::path::{Path, PathBuf};

/// Manages the async file queue: staging, delivery watching, and cleanup.
///
/// This is the core differentiator from LocalSend. Files are staged locally
/// and delivered automatically when the target device comes online.
pub struct QueueService {
    staging_base: PathBuf,
}

impl QueueService {
    pub fn new(staging_base: PathBuf) -> Self {
        std::fs::create_dir_all(&staging_base).ok();
        Self { staging_base }
    }

    /// Stage a file for async delivery to a target device.
    ///
    /// 1. Copies the file to staging/{queue_id}/
    /// 2. Creates a queue entry in the database
    /// 3. Returns the queue_id for tracking
    pub fn enqueue(
        &self,
        db: &rusqlite::Connection,
        source_path: &Path,
        target_device_id: &str,
        file_name: &str,
        file_size: u64,
        sha256: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let queue_id = uuid::Uuid::new_v4().to_string();

        // Create staging directory for this queue entry
        let staging_dir = self.staging_base.join(&queue_id);
        std::fs::create_dir_all(&staging_dir)?;

        // Copy file to staging (in production: encrypt here with group key)
        let staged_path = staging_dir.join(file_name);
        std::fs::copy(source_path, &staged_path)?;

        // Calculate expiry (default: 7 days from now)
        let expires_at = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(DEFAULT_QUEUE_TTL_DAYS))
            .unwrap()
            .to_rfc3339();

        // Insert queue entry
        db.execute(
            "INSERT INTO file_queue (queue_id, target_device_id, file_name, file_size, sha256, staging_path, status, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![
                queue_id,
                target_device_id,
                file_name,
                file_size as i64,
                sha256,
                staged_path.to_string_lossy().to_string(),
                expires_at,
            ],
        )?;

        log::info!(
            "File queued: {} → {} (queue_id: {}, expires: {})",
            file_name, target_device_id, queue_id, expires_at
        );

        Ok(queue_id)
    }

    /// Get all pending queue entries for a specific device.
    /// Called when the delivery watcher detects a device coming online.
    pub fn get_pending_for_device(
        &self,
        db: &rusqlite::Connection,
        device_id: &str,
    ) -> Result<Vec<QueueEntry>, Box<dyn std::error::Error + Send + Sync>> {
        let mut stmt = db.prepare(
            "SELECT queue_id, target_device_id, file_name, file_size, mime_type, sha256,
                    staging_path, status, retry_count, created_at, expires_at,
                    delivered_at, last_attempt_at, error_message
             FROM file_queue
             WHERE target_device_id = ?1 AND status = 'pending'
             ORDER BY created_at ASC"
        )?;

        let entries = stmt.query_map([device_id], |row| {
            Ok(QueueEntry {
                queue_id: row.get(0)?,
                target_device_id: row.get(1)?,
                target_device_name: None,
                file_name: row.get(2)?,
                file_size: row.get::<_, i64>(3)? as u64,
                mime_type: row.get(4)?,
                sha256: row.get(5)?,
                staging_path: row.get(6)?,
                status: QueueStatus::Pending,
                retry_count: row.get(8)?,
                created_at: row.get(9)?,
                expires_at: row.get(10)?,
                delivered_at: row.get(11)?,
                last_attempt_at: row.get(12)?,
                error_message: row.get(13)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

        Ok(entries)
    }

    /// Get all queue entries (for the UI).
    pub fn get_all_entries(
        &self,
        db: &rusqlite::Connection,
    ) -> Result<Vec<QueueEntry>, Box<dyn std::error::Error + Send + Sync>> {
        let mut stmt = db.prepare(
            "SELECT q.queue_id, q.target_device_id, q.file_name, q.file_size, q.mime_type,
                    q.sha256, q.staging_path, q.status, q.retry_count, q.created_at,
                    q.expires_at, q.delivered_at, q.last_attempt_at, q.error_message,
                    d.name as device_name
             FROM file_queue q
             LEFT JOIN paired_devices d ON q.target_device_id = d.device_id
             ORDER BY
                CASE q.status
                    WHEN 'delivering' THEN 0
                    WHEN 'pending' THEN 1
                    WHEN 'failed' THEN 2
                    WHEN 'delivered' THEN 3
                    ELSE 4
                END,
                q.created_at DESC"
        )?;

        let entries = stmt.query_map([], |row| {
            let status_str: String = row.get(7)?;
            let status = match status_str.as_str() {
                "pending" => QueueStatus::Pending,
                "delivering" => QueueStatus::Delivering,
                "delivered" => QueueStatus::Delivered,
                "failed" => QueueStatus::Failed,
                "expired" => QueueStatus::Expired,
                "cancelled" => QueueStatus::Cancelled,
                _ => QueueStatus::Failed,
            };

            Ok(QueueEntry {
                queue_id: row.get(0)?,
                target_device_id: row.get(1)?,
                target_device_name: row.get(14)?,
                file_name: row.get(2)?,
                file_size: row.get::<_, i64>(3)? as u64,
                mime_type: row.get(4)?,
                sha256: row.get(5)?,
                staging_path: row.get(6)?,
                status,
                retry_count: row.get(8)?,
                created_at: row.get(9)?,
                expires_at: row.get(10)?,
                delivered_at: row.get(11)?,
                last_attempt_at: row.get(12)?,
                error_message: row.get(13)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

        Ok(entries)
    }

    /// Get queue statistics for the UI dashboard.
    pub fn get_stats(
        &self,
        db: &rusqlite::Connection,
    ) -> Result<QueueStats, Box<dyn std::error::Error + Send + Sync>> {
        let pending_count: u32 = db.query_row(
            "SELECT COUNT(*) FROM file_queue WHERE status = 'pending'",
            [], |row| row.get(0),
        )?;

        let pending_size: i64 = db.query_row(
            "SELECT COALESCE(SUM(file_size), 0) FROM file_queue WHERE status = 'pending'",
            [], |row| row.get(0),
        )?;

        let delivering_count: u32 = db.query_row(
            "SELECT COUNT(*) FROM file_queue WHERE status = 'delivering'",
            [], |row| row.get(0),
        )?;

        let delivered_today: u32 = db.query_row(
            "SELECT COUNT(*) FROM file_queue WHERE status = 'delivered' AND delivered_at >= datetime('now', '-1 day')",
            [], |row| row.get(0),
        )?;

        let failed_count: u32 = db.query_row(
            "SELECT COUNT(*) FROM file_queue WHERE status = 'failed'",
            [], |row| row.get(0),
        )?;

        Ok(QueueStats {
            pending_count,
            pending_size_bytes: pending_size as u64,
            delivering_count,
            delivered_today,
            failed_count,
        })
    }

    /// Mark a queue entry as "delivering" (transfer in progress).
    pub fn mark_delivering(
        &self,
        db: &rusqlite::Connection,
        queue_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        db.execute(
            "UPDATE file_queue SET status = 'delivering', last_attempt_at = datetime('now')
             WHERE queue_id = ?1 AND status = 'pending'",
            [queue_id],
        )?;
        Ok(())
    }

    /// Mark a queue entry as successfully delivered.
    /// Cleans up the staged file.
    pub fn mark_delivered(
        &self,
        db: &rusqlite::Connection,
        queue_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Get the staging path before updating
        let staging_path: String = db.query_row(
            "SELECT staging_path FROM file_queue WHERE queue_id = ?1",
            [queue_id],
            |row| row.get(0),
        )?;

        // Update status
        db.execute(
            "UPDATE file_queue SET status = 'delivered', delivered_at = datetime('now')
             WHERE queue_id = ?1",
            [queue_id],
        )?;

        // Clean up staged file
        let path = PathBuf::from(&staging_path);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        // Remove the staging directory if empty
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }

        log::info!("Queue entry delivered and cleaned up: {}", queue_id);
        Ok(())
    }

    /// Mark a queue entry as failed. Increments retry count.
    /// If max retries reached, status stays "failed" permanently.
    pub fn mark_failed(
        &self,
        db: &rusqlite::Connection,
        queue_id: &str,
        error: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Check retry count
        let (retry_count, max_retries): (i32, i32) = db.query_row(
            "SELECT retry_count, max_retries FROM file_queue WHERE queue_id = ?1",
            [queue_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let new_status = if retry_count + 1 >= max_retries {
            "failed" // permanent failure
        } else {
            "pending" // back to pending for retry
        };

        db.execute(
            "UPDATE file_queue SET status = ?1, retry_count = retry_count + 1,
             last_attempt_at = datetime('now'), error_message = ?2
             WHERE queue_id = ?3",
            params![new_status, error, queue_id],
        )?;

        log::warn!(
            "Queue entry {} failed (attempt {}/{}): {}",
            queue_id, retry_count + 1, max_retries, error
        );
        Ok(())
    }

    /// Cancel a pending queue entry. Cleans up staged file.
    pub fn cancel(
        &self,
        db: &rusqlite::Connection,
        queue_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let staging_path: String = db.query_row(
            "SELECT staging_path FROM file_queue WHERE queue_id = ?1",
            [queue_id],
            |row| row.get(0),
        )?;

        db.execute(
            "UPDATE file_queue SET status = 'cancelled' WHERE queue_id = ?1 AND status IN ('pending', 'failed')",
            [queue_id],
        )?;

        // Clean up staged file
        let path = PathBuf::from(&staging_path);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }

        log::info!("Queue entry cancelled: {}", queue_id);
        Ok(())
    }

    /// Expire old queue entries. Called periodically by the delivery watcher.
    pub fn expire_old_entries(
        &self,
        db: &rusqlite::Connection,
    ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
        // Find expired entries
        let mut stmt = db.prepare(
            "SELECT queue_id, staging_path FROM file_queue
             WHERE status = 'pending' AND expires_at < datetime('now')"
        )?;

        let expired: Vec<(String, String)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

        let count = expired.len() as u32;

        for (queue_id, staging_path) in &expired {
            db.execute(
                "UPDATE file_queue SET status = 'expired' WHERE queue_id = ?1",
                [queue_id],
            )?;

            // Clean up staged file
            let path = PathBuf::from(staging_path);
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            if let Some(parent) = path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }

        if count > 0 {
            log::info!("Expired {} queue entries", count);
        }
        Ok(count)
    }

    /// Clean up delivered entries older than 24 hours (remove from DB).
    pub fn cleanup_delivered(
        &self,
        db: &rusqlite::Connection,
    ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
        let deleted = db.execute(
            "DELETE FROM file_queue
             WHERE status IN ('delivered', 'expired', 'cancelled')
             AND (delivered_at < datetime('now', '-1 day')
                  OR created_at < datetime('now', '-7 days'))",
            [],
        )?;
        Ok(deleted as u32)
    }
}
