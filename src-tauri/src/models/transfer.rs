use serde::{Deserialize, Serialize};

/// Transfer direction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Sent,
    Received,
}

/// Transfer status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TransferStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

/// Default chunk size: 1 MB
pub const DEFAULT_CHUNK_SIZE: u64 = 1_048_576;

/// Default file size cap: 2 GB
pub const DEFAULT_FILE_SIZE_CAP: u64 = 2_147_483_648;

/// Maximum parallel chunks in flight
pub const MAX_PARALLEL_CHUNKS: usize = 4;

/// A file transfer record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub transfer_id: String,
    pub direction: TransferDirection,
    pub peer_device_id: String,
    pub peer_device_name: Option<String>,
    pub file_name: String,
    pub file_size: u64,
    pub mime_type: Option<String>,
    pub sha256: String,
    pub status: TransferStatus,
    pub chunks_total: u32,
    pub chunks_completed: u32,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub error_message: Option<String>,
}

impl Transfer {
    /// Progress as a percentage (0-100)
    pub fn progress_percent(&self) -> f32 {
        if self.chunks_total == 0 {
            return 0.0;
        }
        (self.chunks_completed as f32 / self.chunks_total as f32) * 100.0
    }
}

/// File metadata sent in a transfer request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub name: String,
    pub size: u64,
    pub mime_type: Option<String>,
    pub sha256: String,
    pub chunk_count: u32,
    pub chunk_size: u64,
}

/// Protocol messages exchanged between devices
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProtocolMessage {
    /// Sender → Receiver: request to transfer a file
    TransferRequest {
        transfer_id: String,
        sender_device_id: String,
        file: FileMetadata,
    },
    /// Receiver → Sender: accept the transfer
    TransferAccept {
        transfer_id: String,
    },
    /// Receiver → Sender: reject the transfer
    TransferReject {
        transfer_id: String,
        reason: String,
    },
    /// Sender → Receiver: a chunk of file data
    Chunk {
        transfer_id: String,
        index: u32,
        data: Vec<u8>, // encrypted chunk data
        nonce: Vec<u8>,
    },
    /// Receiver → Sender: acknowledge received chunks
    ChunkAck {
        transfer_id: String,
        indices: Vec<u32>,
    },
    /// Sender → Receiver: all chunks sent
    TransferComplete {
        transfer_id: String,
    },
    /// Receiver → Sender: file hash verified
    TransferVerified {
        transfer_id: String,
        sha256_match: bool,
    },
    /// Either → Either: request to resume an interrupted transfer
    TransferResume {
        transfer_id: String,
        completed_chunks: Vec<u32>,
    },
}

/// Storage statistics returned to the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    pub folder_path: String,
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub file_size_cap_bytes: u64,
    pub file_count: u32,
}
