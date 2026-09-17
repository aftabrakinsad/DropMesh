/// DropMesh Transfer Engine
///
/// All device-to-device communication runs over QUIC (via quinn), which
/// includes TLS 1.3 in every connection:
///   - authenticated encryption on the wire
///   - forward secrecy at the transport layer
///   - congestion control that behaves well on Wi-Fi
///
/// A single QUIC stream carries either a pairing handshake or a file
/// transfer. `handle_incoming_stream` peeks at the first message and
/// routes accordingly.
///
/// Per-transfer security, layered on top of TLS:
///   1. session key = HKDF(group_key, transfer_id)  → forward secrecy
///   2. each chunk encrypted with AES-256-GCM using the session key
///   3. HMAC-SHA256 over the whole file proves it came from a trusted device

use crate::models::transfer::*;
use crate::services::pairing_handshake::PairMessage;
use crate::services::security::SecurityModule;
use quinn::{ClientConfig, Endpoint, ServerConfig};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Progress event pushed to the frontend during a transfer
#[derive(Debug, Clone, Serialize)]
pub struct TransferProgress {
    pub transfer_id: String,
    pub chunks_completed: u32,
    pub chunks_total: u32,
    pub bytes_transferred: u64,
    pub speed_bytes_per_sec: u64,
}

/// Wire protocol for file transfers
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WireMessage {
    TransferRequest {
        transfer_id: String,
        sender_device_id: String,
        file_name: String,
        file_size: u64,
        sha256: String,
        hmac_tag: Vec<u8>,
        chunk_count: u32,
        chunk_size: u64,
    },
    TransferAccept { transfer_id: String },
    TransferReject { transfer_id: String, reason: String },
    Chunk {
        transfer_id: String,
        index: u32,
        data: Vec<u8>,
        nonce: Vec<u8>,
    },
    ChunkAck { transfer_id: String, indices: Vec<u32> },
    TransferComplete { transfer_id: String },
    TransferVerified { transfer_id: String, sha256_match: bool },
}

struct ActiveTransfer {
    #[allow(dead_code)]
    transfer_id: String,
    #[allow(dead_code)]
    file_path: PathBuf,
    #[allow(dead_code)]
    file_size: u64,
    cancel_tx: Option<mpsc::Sender<()>>,
}

pub struct TransferEngine {
    active: HashMap<String, ActiveTransfer>,
    listener_port: Option<u16>,
}

impl TransferEngine {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self { active: HashMap::new(), listener_port: None })
    }

    // ── QUIC configuration ───────────────────────────────────────────────────

    /// Self-signed certificate for this device's QUIC endpoint.
    /// Regenerated each launch; peers are authenticated by the pairing PIN
    /// and the HMAC on each file rather than by a CA hierarchy.
    pub fn generate_self_signed_cert() -> Result<
        (CertificateDer<'static>, PrivateKeyDer<'static>),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let cert = generate_simple_self_signed(vec!["dropmesh.local".to_string()])
            .map_err(|e| format!("Cert generation failed: {}", e))?;
        let cert_der = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivateKeyDer::try_from(cert.key_pair.serialize_der())
            .map_err(|e| format!("Key serialization failed: {}", e))?;
        Ok((cert_der, key_der))
    }

    fn build_server_config(
        cert: CertificateDer<'static>,
        key: PrivateKeyDer<'static>,
    ) -> Result<ServerConfig, Box<dyn std::error::Error + Send + Sync>> {
        let mut server_crypto = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .map_err(|e| format!("TLS server config failed: {}", e))?;
        server_crypto.alpn_protocols = vec![b"dropmesh/1".to_vec()];

        let mut server_config = ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
                .map_err(|e| format!("QUIC server config failed: {}", e))?,
        ));

        let mut transport = quinn::TransportConfig::default();
        transport.max_concurrent_bidi_streams(16u32.into());
        transport.keep_alive_interval(Some(std::time::Duration::from_secs(5)));
        server_config.transport_config(Arc::new(transport));

        Ok(server_config)
    }

    // ── Listener ─────────────────────────────────────────────────────────────

    /// Bind the QUIC endpoint and accept incoming streams.
    /// Each stream is routed to either pairing or file transfer.
    pub async fn start_listener(
        &mut self,
        app_handle: tauri::AppHandle,
        db: Arc<tokio::sync::Mutex<crate::db::Database>>,
        storage: Arc<tokio::sync::Mutex<crate::services::storage::StorageManager>>,
    ) -> Result<u16, Box<dyn std::error::Error + Send + Sync>> {
        let (cert, key) = Self::generate_self_signed_cert()?;
        let server_config = Self::build_server_config(cert, key)?;

        // Bind to port 0 so the OS assigns a free port. Hard-coding a port
        // breaks the moment two instances run on one machine, and the real
        // port is what gets advertised over mDNS anyway.
        let addr: SocketAddr = "0.0.0.0:0".parse()?;
        let endpoint = Endpoint::server(server_config, addr)
            .map_err(|e| format!("Failed to bind QUIC endpoint: {}", e))?;

        let port = endpoint.local_addr()?.port();
        self.listener_port = Some(port);
        log::info!("QUIC listener started on port {}", port);

        tauri::async_runtime::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let app_handle = app_handle.clone();
                let db = db.clone();
                let storage = storage.clone();

                tauri::async_runtime::spawn(async move {
                    match incoming.await {
                        Ok(conn) => {
                            log::info!("Connection from {}", conn.remote_address());
                            loop {
                                match conn.accept_bi().await {
                                    Ok((send, recv)) => {
                                        handle_incoming_stream(
                                            send, recv,
                                            app_handle.clone(),
                                            db.clone(),
                                            storage.clone(),
                                        ).await;
                                    }
                                    Err(e) => {
                                        log::debug!("Connection closed: {}", e);
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => log::error!("Connection error: {}", e),
                    }
                });
            }
        });

        Ok(port)
    }

    // ── Sending a file ───────────────────────────────────────────────────────

    pub async fn send_file(
        &mut self,
        file_path: PathBuf,
        peer_host: &str,
        peer_port: u16,
        group_key: &[u8],
        _peer_cert: Option<CertificateDer<'static>>,
        sender_device_id: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let transfer_id = uuid::Uuid::new_v4().to_string();

        let file_bytes = tokio::fs::read(&file_path).await
            .map_err(|e| format!("Cannot read file: {}", e))?;
        let file_size = file_bytes.len() as u64;
        let file_name = file_path.file_name()
            .unwrap_or_default().to_string_lossy().to_string();

        // Fresh session key for this transfer only
        let session_key = SecurityModule::derive_session_key(group_key, &transfer_id)?;
        let sha256 = SecurityModule::hash_file(&file_bytes);
        let hmac_tag = SecurityModule::compute_file_hmac(&session_key, &file_bytes);
        let chunk_count = ((file_size + DEFAULT_CHUNK_SIZE - 1) / DEFAULT_CHUNK_SIZE) as u32;

        let conn = open_connection(peer_host, peer_port).await?;
        let (mut send, mut recv) = conn.open_bi().await
            .map_err(|e| format!("Stream open failed: {}", e))?;

        write_message(&mut send, &WireMessage::TransferRequest {
            transfer_id: transfer_id.clone(),
            sender_device_id: sender_device_id.to_string(),
            file_name: file_name.clone(),
            file_size,
            sha256,
            hmac_tag,
            chunk_count,
            chunk_size: DEFAULT_CHUNK_SIZE,
        }).await?;

        match read_message(&mut recv).await? {
            WireMessage::TransferAccept { .. } => {
                log::info!("Transfer {} accepted", &transfer_id[..8]);
            }
            WireMessage::TransferReject { reason, .. } => {
                return Err(format!("Rejected: {}", reason).into());
            }
            _ => return Err("Unexpected response to TransferRequest".into()),
        }

        for chunk_idx in 0..chunk_count {
            let start = (chunk_idx as u64 * DEFAULT_CHUNK_SIZE) as usize;
            let end = ((chunk_idx as u64 + 1) * DEFAULT_CHUNK_SIZE).min(file_size) as usize;

            let (ciphertext, nonce) = SecurityModule::encrypt_chunk(
                &session_key, &transfer_id, chunk_idx, &file_bytes[start..end],
            )?;

            write_message(&mut send, &WireMessage::Chunk {
                transfer_id: transfer_id.clone(),
                index: chunk_idx,
                data: ciphertext,
                nonce,
            }).await?;

            let is_window_end = (chunk_idx + 1) % MAX_PARALLEL_CHUNKS as u32 == 0;
            let is_last = chunk_idx + 1 >= chunk_count;
            if is_window_end || is_last {
                match read_message(&mut recv).await? {
                    WireMessage::ChunkAck { indices, .. } => {
                        log::debug!("ACK {:?}", indices);
                    }
                    _ => return Err("Expected ChunkAck".into()),
                }
            }
        }

        write_message(&mut send, &WireMessage::TransferComplete {
            transfer_id: transfer_id.clone(),
        }).await?;

        match read_message(&mut recv).await? {
            WireMessage::TransferVerified { sha256_match, .. } => {
                if !sha256_match {
                    return Err("Receiver reported verification failure".into());
                }
                log::info!("Transfer {} verified", &transfer_id[..8]);
            }
            _ => return Err("Expected TransferVerified".into()),
        }

        let _ = send.finish();
        Ok(transfer_id)
    }

    pub fn cancel_transfer(
        &mut self,
        transfer_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(t) = self.active.remove(transfer_id) {
            if let Some(tx) = t.cancel_tx { let _ = tx.try_send(()); }
            log::info!("Transfer {} cancelled", transfer_id);
        }
        Ok(())
    }

    pub fn listener_port(&self) -> Option<u16> { self.listener_port }
}

// ── Stream router ────────────────────────────────────────────────────────────

/// Reads the first message on a stream and decides whether it is a pairing
/// handshake or a file transfer, then hands off accordingly.
pub async fn handle_incoming_stream(
    send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    app_handle: tauri::AppHandle,
    db: Arc<tokio::sync::Mutex<crate::db::Database>>,
    storage: Arc<tokio::sync::Mutex<crate::services::storage::StorageManager>>,
) {
    let mut len_buf = [0u8; 4];
    if recv.read_exact(&mut len_buf).await.is_err() { return; }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        log::error!("First message too large: {}", len);
        return;
    }
    let mut buf = vec![0u8; len];
    if recv.read_exact(&mut buf).await.is_err() { return; }

    if let Ok(PairMessage::PairRequest { device_id, name, device_type, public_key, pin }) =
        serde_json::from_slice::<PairMessage>(&buf)
    {
        handle_pair_request(send, device_id, name, device_type, public_key, pin, db, app_handle).await;
        return;
    }

    if let Ok(wire) = serde_json::from_slice::<WireMessage>(&buf) {
        handle_incoming_transfer(send, recv, wire, app_handle, db, storage).await;
        return;
    }

    log::error!("Unrecognized message on incoming stream");
}

// ── Pairing handshake (Device A side) ────────────────────────────────────────

/// Validate the PIN, store the peer's real identity, and reply with ours.
/// Storing the real device_id is what lets mDNS match the device later and
/// show it as online.
#[allow(clippy::too_many_arguments)]
async fn handle_pair_request(
    mut send: quinn::SendStream,
    peer_id: String,
    peer_name: String,
    peer_type: String,
    peer_pubkey_b64: String,
    pin: String,
    db: Arc<tokio::sync::Mutex<crate::db::Database>>,
    app_handle: tauri::AppHandle,
) {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use tauri::Emitter;

    log::info!("Pairing request from {} ({})", peer_name, peer_id);

    let db_guard = db.lock().await;

    let active_pin: String = db_guard.conn().query_row(
        "SELECT value FROM storage_config WHERE key = 'active_pairing_pin'",
        [], |row| row.get(0),
    ).unwrap_or_default();

    if active_pin.is_empty() || active_pin != pin {
        log::warn!("Pairing rejected — PIN mismatch from {}", peer_id);
        let _ = write_pair_message(&mut send, &PairMessage::PairRejected {
            reason: "Incorrect pairing code".to_string(),
        }).await;
        return;
    }

    let expires_str: String = db_guard.conn().query_row(
        "SELECT value FROM storage_config WHERE key = 'active_pairing_expires'",
        [], |row| row.get(0),
    ).unwrap_or_default();

    if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&expires_str) {
        if chrono::Utc::now() > expires {
            log::warn!("Pairing rejected — code expired");
            let _ = write_pair_message(&mut send, &PairMessage::PairRejected {
                reason: "Pairing code expired — generate a new one".to_string(),
            }).await;
            return;
        }
    }

    let (my_id, my_name, my_type, my_pubkey): (String, String, String, Vec<u8>) =
        match db_guard.conn().query_row(
            "SELECT id, name, type, public_key FROM device_self LIMIT 1",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ) {
            Ok(v) => v,
            Err(e) => { log::error!("No device identity: {}", e); return; }
        };

    let group_id: String = db_guard.conn().query_row(
        "SELECT value FROM storage_config WHERE key = 'active_pairing_group'",
        [], |row| row.get(0),
    ).unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());

    let group_key = match SecurityModule::derive_group_key_from_pairing_code(&pin) {
        Ok(k) => k,
        Err(e) => { log::error!("Key derivation failed: {}", e); return; }
    };

    let peer_pubkey_bytes = STANDARD.decode(&peer_pubkey_b64).unwrap_or_default();

    let _ = db_guard.conn().execute(
        "INSERT OR REPLACE INTO trust_group (group_id, group_key_encrypted) VALUES (?1, ?2)",
        rusqlite::params![group_id, group_key.clone()],
    );

    if let Err(e) = db_guard.conn().execute(
        "INSERT OR REPLACE INTO paired_devices
         (device_id, name, type, public_key, group_id, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        rusqlite::params![peer_id, peer_name, peer_type, peer_pubkey_bytes, group_id],
    ) {
        log::error!("Failed to store peer: {}", e);
        return;
    }

    // PIN is single-use
    let _ = db_guard.conn().execute(
        "DELETE FROM storage_config WHERE key IN
         ('active_pairing_pin','active_pairing_expires','active_pairing_group')",
        [],
    );

    drop(db_guard);

    let emojis = SecurityModule::derive_verification_emojis(&group_key).join(" ");

    if let Err(e) = write_pair_message(&mut send, &PairMessage::PairResponse {
        device_id: my_id,
        name: my_name,
        device_type: my_type,
        public_key: STANDARD.encode(&my_pubkey),
        group_id,
        verification_emojis: emojis.clone(),
    }).await {
        log::error!("Failed to send pair response: {}", e);
        return;
    }

    log::info!("Paired with {} ({}) — emojis: {}", peer_name, peer_id, emojis);

    let _ = app_handle.emit("on_pairing_complete", serde_json::json!({
        "peer_device_id": peer_id,
        "peer_name": peer_name,
        "verification_emojis": emojis,
    }));
}

/// Device B side: open a connection, send the PairRequest, await the response.
pub async fn send_pair_request(
    peer_host: &str,
    peer_port: u16,
    request: PairMessage,
) -> Result<PairMessage, Box<dyn std::error::Error + Send + Sync>> {
    let conn = open_connection(peer_host, peer_port).await?;
    let (mut send, mut recv) = conn.open_bi().await
        .map_err(|e| format!("Stream open failed: {}", e))?;

    write_pair_message(&mut send, &request).await?;

    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 1024 * 1024 {
        return Err("Pairing response too large".into());
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;

    let response: PairMessage = serde_json::from_slice(&buf)?;
    let _ = send.finish();
    Ok(response)
}

// ── Receiving a file ─────────────────────────────────────────────────────────

/// Decrypt chunks, reassemble, then verify BOTH the SHA-256 (integrity) and
/// the HMAC (authenticity). A file failing either check is deleted, not saved.
async fn handle_incoming_transfer(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    first_message: WireMessage,
    app_handle: tauri::AppHandle,
    db: Arc<tokio::sync::Mutex<crate::db::Database>>,
    storage: Arc<tokio::sync::Mutex<crate::services::storage::StorageManager>>,
) {
    use tauri::Emitter;

    let (transfer_id, sender_id, file_name, file_size, sha256, hmac_tag, chunk_count) =
        match first_message {
            WireMessage::TransferRequest {
                transfer_id, sender_device_id, file_name,
                file_size, sha256, hmac_tag, chunk_count, ..
            } => (transfer_id, sender_device_id, file_name,
                  file_size, sha256, hmac_tag, chunk_count),
            _ => { log::error!("Expected TransferRequest"); return; }
        };

    log::info!("Incoming: {} ({} bytes) from {}", file_name, file_size, sender_id);

    let group_key = {
        let db = db.lock().await;
        match get_group_key_for_device(&db, &sender_id) {
            Some(k) => k,
            None => {
                log::error!("Sender {} not in trust group — rejecting", sender_id);
                let _ = write_message(&mut send, &WireMessage::TransferReject {
                    transfer_id, reason: "Sender not in trust group".to_string(),
                }).await;
                return;
            }
        }
    };

    let session_key = match SecurityModule::derive_session_key(&group_key, &transfer_id) {
        Ok(k) => k,
        Err(e) => { log::error!("Session key derivation failed: {}", e); return; }
    };

    let staging_path = {
        let storage = storage.lock().await;
        if !storage.can_accept_file(file_size) {
            let _ = write_message(&mut send, &WireMessage::TransferReject {
                transfer_id, reason: "Insufficient storage".to_string(),
            }).await;
            return;
        }
        storage.incoming_path(&transfer_id)
    };
    tokio::fs::create_dir_all(&staging_path).await.ok();

    if write_message(&mut send, &WireMessage::TransferAccept {
        transfer_id: transfer_id.clone(),
    }).await.is_err() {
        return;
    }

    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut ack_buffer: Vec<u32> = Vec::new();

    loop {
        let msg = match read_message(&mut recv).await {
            Ok(m) => m,
            Err(e) => { log::error!("Chunk read error: {}", e); return; }
        };

        match msg {
            WireMessage::Chunk { index, mut data, nonce, .. } => {
                match SecurityModule::decrypt_chunk(&session_key, &nonce, &mut data) {
                    Ok(plaintext) => {
                        if index as usize == chunks.len() {
                            chunks.push(plaintext);
                        }
                        ack_buffer.push(index);

                        // The sender blocks for an ACK at the end of each
                        // window and after the final chunk. Both sides must
                        // use the same rule or the transfer deadlocks.
                        let window_end = (index + 1) % MAX_PARALLEL_CHUNKS as u32 == 0;
                        let last_chunk = index + 1 >= chunk_count;

                        if window_end || last_chunk {
                            let _ = write_message(&mut send, &WireMessage::ChunkAck {
                                transfer_id: transfer_id.clone(),
                                indices: ack_buffer.drain(..).collect(),
                            }).await;
                        }
                    }
                    Err(e) => { log::error!("Chunk {} decryption failed: {}", index, e); return; }
                }
            }
            WireMessage::TransferComplete { .. } => {
                // Normally the last-chunk ACK already went out; this covers
                // a zero-byte file or any stragglers.
                if !ack_buffer.is_empty() {
                    let _ = write_message(&mut send, &WireMessage::ChunkAck {
                        transfer_id: transfer_id.clone(),
                        indices: ack_buffer.drain(..).collect(),
                    }).await;
                }
                break;
            }
            _ => { log::error!("Unexpected message during transfer"); return; }
        }
    }

    let full_file: Vec<u8> = chunks.into_iter().flatten().collect();

    let sha_ok = SecurityModule::hash_file(&full_file) == sha256;
    let hmac_ok = SecurityModule::verify_file_hmac(&session_key, &full_file, &hmac_tag).is_ok();

    if !sha_ok { log::error!("SHA-256 mismatch for {}", &transfer_id[..8]); }
    if !hmac_ok { log::error!("HMAC failed for {} — possible tampering", &transfer_id[..8]); }

    let _ = write_message(&mut send, &WireMessage::TransferVerified {
        transfer_id: transfer_id.clone(),
        sha256_match: sha_ok && hmac_ok,
    }).await;

    if !(sha_ok && hmac_ok) {
        let _ = tokio::fs::remove_dir_all(&staging_path).await;
        log::error!("Transfer {} rejected — file discarded", &transfer_id[..8]);
        return;
    }

    let dest_path = {
        let storage = storage.lock().await;
        storage.received_path(&file_name)
    };

    if let Err(e) = tokio::fs::write(&dest_path, &full_file).await {
        log::error!("Failed to save received file: {}", e);
        return;
    }
    let _ = tokio::fs::remove_dir_all(&staging_path).await;

    log::info!("Saved {} to {:?}", file_name, dest_path);

    let _ = app_handle.emit("on_incoming_file", serde_json::json!({
        "transfer_id": transfer_id,
        "file_name": file_name,
        "file_size": file_size,
        "from_device_id": sender_id,
        "saved_path": dest_path.to_string_lossy(),
        "status": "delivered",
    }));
}

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn open_connection(
    peer_host: &str,
    peer_port: u16,
) -> Result<quinn::Connection, Box<dyn std::error::Error + Send + Sync>> {
    let bind_addr: SocketAddr = "0.0.0.0:0".parse()?;
    let mut endpoint = Endpoint::client(bind_addr)
        .map_err(|e| format!("QUIC client bind failed: {}", e))?;
    endpoint.set_default_client_config(build_client_config()?);

    let peer_addr: SocketAddr = format!("{}:{}", peer_host, peer_port).parse()
        .map_err(|e| format!("Invalid address {}:{} — {}", peer_host, peer_port, e))?;

    let conn = endpoint
        .connect(peer_addr, "dropmesh.local")
        .map_err(|e| format!("QUIC connect failed: {}", e))?
        .await
        .map_err(|e| format!("Could not reach {}: {}", peer_addr, e))?;

    Ok(conn)
}

async fn write_message(
    stream: &mut quinn::SendStream,
    msg: &WireMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let json = serde_json::to_vec(msg)?;
    stream.write_all(&(json.len() as u32).to_be_bytes()).await?;
    stream.write_all(&json).await?;
    Ok(())
}

async fn read_message(
    stream: &mut quinn::RecvStream,
) -> Result<WireMessage, Box<dyn std::error::Error + Send + Sync>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        return Err(format!("Message too large: {} bytes", len).into());
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

async fn write_pair_message(
    stream: &mut quinn::SendStream,
    msg: &PairMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let json = serde_json::to_vec(msg)?;
    stream.write_all(&(json.len() as u32).to_be_bytes()).await?;
    stream.write_all(&json).await?;
    Ok(())
}

fn get_group_key_for_device(db: &crate::db::Database, device_id: &str) -> Option<Vec<u8>> {
    db.conn().query_row(
        "SELECT tg.group_key_encrypted FROM trust_group tg
         INNER JOIN paired_devices pd ON pd.group_id = tg.group_id
         WHERE pd.device_id = ?1",
        [device_id],
        |row| row.get(0),
    ).ok()
}

/// Peer certificates are self-signed and regenerated each launch, so TLS
/// chain validation is skipped. Authenticity comes from the pairing PIN and
/// the per-file HMAC instead. Pinning peer certs at pairing time would be a
/// worthwhile hardening step later.
fn build_client_config() -> Result<ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    #[derive(Debug)]
    struct AcceptAnyCert;

    impl rustls::client::danger::ServerCertVerifier for AcceptAnyCert {
        fn verify_server_cert(
            &self, _: &CertificateDer, _: &[CertificateDer],
            _: &rustls::pki_types::ServerName, _: &[u8],
            _: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self, _: &[u8], _: &CertificateDer, _: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self, _: &[u8], _: &CertificateDer, _: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms.supported_schemes()
        }
    }

    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
        .with_no_client_auth();
    crypto.alpn_protocols = vec![b"dropmesh/1".to_vec()];

    Ok(ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
            .map_err(|e| format!("QUIC client config failed: {}", e))?,
    )))
}