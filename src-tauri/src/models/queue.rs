use serde::{Deserialize, Serialize};

/// Default TTL for queued files: 7 days
pub const DEFAULT_QUEUE_TTL_DAYS: i64 = 7;

/// Maximum retry attempts before giving up
pub const DEFAULT_MAX_RETRIES: i32 = 10;

/// Queue entry status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum QueueStatus {
    Pending,
    Delivering,
    Delivered,
    Failed,
    Expired,
    Cancelled,
}

impl std::fmt::Display for QueueStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueStatus::Pending => write!(f, "pending"),
            QueueStatus::Delivering => write!(f, "delivering"),
            QueueStatus::Delivered => write!(f, "delivered"),
            QueueStatus::Failed => write!(f, "failed"),
            QueueStatus::Expired => write!(f, "expired"),
            QueueStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// A file waiting in the async delivery queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueEntry {
    pub queue_id: String,
    pub target_device_id: String,
    pub target_device_name: Option<String>,
    pub file_name: String,
    pub file_size: u64,
    pub mime_type: Option<String>,
    pub sha256: String,
    pub staging_path: String,
    pub status: QueueStatus,
    pub retry_count: i32,
    pub created_at: String,
    pub expires_at: String,
    pub delivered_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub error_message: Option<String>,
}

/// Summary stats for the queue (shown in UI)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStats {
    pub pending_count: u32,
    pub pending_size_bytes: u64,
    pub delivering_count: u32,
    pub delivered_today: u32,
    pub failed_count: u32,
}
