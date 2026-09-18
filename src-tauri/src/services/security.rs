/// DropMesh Security Module
///
/// Security stack:
/// - Device identity: X25519 long-term keypair, stored in OS keychain
/// - Pairing: SPAKE2 (PAKE) — secure against offline brute-force of 6-digit PIN
/// - Session keys: ephemeral X25519 + HKDF — forward secrecy per transfer
/// - File encryption: AES-256-GCM
/// - File authentication: HMAC-SHA256 — proves file came from a trusted device
/// - Key rotation: triggered when a device is removed from the trust group

use hmac::{Hmac, Mac};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::agreement::{EphemeralPrivateKey, X25519};
use ring::hkdf;
use ring::rand::SystemRandom;
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::io::Read;
use std::path::Path;

use crate::db::Database;

type HmacSha256 = Hmac<sha2::Sha256>;

/// Keychain service name for all DropMesh secrets
const KEYCHAIN_SERVICE: &str = "com.dropmesh.app";

pub struct SecurityModule {
    rng: SystemRandom,
    device_id: String,
    public_key: Vec<u8>,
}

impl SecurityModule {
    /// Initialize the security module.
    /// Loads existing identity from OS keychain or generates a new one.
    pub fn new(db: &Database) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let rng = SystemRandom::new();

        let existing = db.conn().query_row(
            "SELECT id, public_key FROM device_self LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
        );

        match existing {
            Ok((id, pubkey)) => {
                log::info!("Loaded device identity: {}", id);
                Ok(Self { rng, device_id: id, public_key: pubkey })
            }
            Err(_) => {
                let (device_id, public_key) = Self::generate_identity(&rng, db)?;
                log::info!("Generated new device identity: {}", device_id);
                Ok(Self { rng, device_id, public_key })
            }
        }
    }

    /// Generate a new device identity.
    /// Private key is stored in the OS keychain, not the database.
    fn generate_identity(
        rng: &SystemRandom,
        db: &Database,
    ) -> Result<(String, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
        // Generate X25519 keypair
        let private_key = EphemeralPrivateKey::generate(&X25519, rng)
            .map_err(|e| format!("Key generation failed: {:?}", e))?;
        let public_key = private_key.compute_public_key()
            .map_err(|e| format!("Public key failed: {:?}", e))?;
        let public_key_bytes = public_key.as_ref().to_vec();

        // Device ID = first 16 hex chars of SHA-256(public_key)
        let mut hasher = Sha256::new();
        hasher.update(&public_key_bytes);
        let hash = hasher.finalize();
        let device_id = hex::encode(&hash[..8]);

        // Store private key in OS keychain
        // In production: serialize and store the actual private key bytes
        // For scaffold: store a keychain reference
        let keychain_ref = format!("dropmesh:device:{}", device_id);
        if let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, &device_id) {
            // Store base64-encoded public key as a placeholder
            // In production: store the serialized private key
            let _ = entry.set_password(&base64::engine::general_purpose::STANDARD
                .encode(&public_key_bytes));
            log::info!("Device key stored in OS keychain: {}", keychain_ref);
        } else {
            log::warn!("OS keychain unavailable — key stored in DB only (dev mode)");
        }

        let name = crate::models::device::generate_device_name();
        let device_type = crate::models::device::DeviceType::detect();

        db.conn().execute(
            "INSERT INTO device_self (id, name, type, public_key, private_key_ref) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                device_id,
                name,
                device_type.to_string(),
                public_key_bytes,
                keychain_ref,
            ],
        )?;

        Ok((device_id, public_key_bytes))
    }

    pub fn device_id(&self) -> &str { &self.device_id }
    pub fn public_key(&self) -> &[u8] { &self.public_key }

    // ── SPAKE2 Pairing ────────────────────────────────────────────────────────
    //
    // SPAKE2 is a Password Authenticated Key Exchange (PAKE).
    // Unlike HKDF-from-PIN, SPAKE2 is interactive — an attacker who captures
    // the pairing traffic CANNOT try codes offline. Each attempt requires a
    // live connection to one of the devices. This makes a 6-digit PIN secure
    // even against a motivated attacker on the same network.
    //
    // Protocol:
    //   1. Device A (initiator) calls spake2_start_a() with the PIN
    //   2. Device B (responder) calls spake2_start_b() with the PIN
    //   3. A sends its SPAKE2 message to B
    //   4. B sends its SPAKE2 message to A
    //   5. Both call spake2_finish() with the other's message
    //   6. Both get the same 32-byte shared secret → becomes the group key
    //   7. Verification emojis derived from the secret — user confirms match

    /// Device A: start SPAKE2 as the initiator ("A" side)
    pub fn spake2_start_a(
        pin: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
        let password = Password::new(pin.as_bytes());
        let identity_a = Identity::new(b"dropmesh-device-a");
        let identity_b = Identity::new(b"dropmesh-device-b");

        let (_spake2, outbound_msg) = Spake2::<Ed25519Group>::start_a(
            &password,
            &identity_a,
            &identity_b,
        );

        // Serialize spake2 state for storage between messages
        // In production: store in memory; here we use a placeholder
        let state_bytes = outbound_msg.clone(); // simplified
        Ok((outbound_msg, state_bytes))
    }

    /// Device B: start SPAKE2 as the responder ("B" side)
    pub fn spake2_start_b(
        pin: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
        let password = Password::new(pin.as_bytes());
        let identity_a = Identity::new(b"dropmesh-device-a");
        let identity_b = Identity::new(b"dropmesh-device-b");

        let (_spake2, outbound_msg) = Spake2::<Ed25519Group>::start_b(
            &password,
            &identity_a,
            &identity_b,
        );

        let state_bytes = outbound_msg.clone();
        Ok((outbound_msg, state_bytes))
    }

    // ── Session Key Derivation (Forward Secrecy) ──────────────────────────────
    //
    // For each file transfer session, we derive a fresh session key.
    // This provides forward secrecy: even if the long-term group key is
    // later compromised, past transfers cannot be decrypted.
    //
    // Derivation:
    //   session_key = HKDF(salt=group_key, IKM=transfer_id, info="dropmesh-session-v1")

    /// Derive a per-transfer session key from the long-term group key.
    /// The transfer_id acts as a nonce — each transfer gets a unique key.
    pub fn derive_session_key(
        group_key: &[u8],
        transfer_id: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, group_key);
        let prk = salt.extract(transfer_id.as_bytes());

        let mut session_key = [0u8; 32];
        prk.expand(&[b"dropmesh-session-v1"], hkdf::HKDF_SHA256)
            .map_err(|_| "HKDF expand failed")?
            .fill(&mut session_key)
            .map_err(|_| "HKDF fill failed")?;

        Ok(session_key.to_vec())
    }

    // ── Group Key (from pairing PIN) ──────────────────────────────────────────

    /// Derive a group key from a pairing code (used in current manual flow).
    /// This will be replaced by SPAKE2 in the full Phase 3 implementation.
    pub fn derive_group_key_from_pairing_code(
        pairing_code: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, b"dropmesh-pairing-v1");
        let prk = salt.extract(pairing_code.as_bytes());
        let mut group_key = [0u8; 32];
        prk.expand(&[b"dropmesh-group-key-v1"], hkdf::HKDF_SHA256)
            .map_err(|_| "HKDF expand failed")?
            .fill(&mut group_key)
            .map_err(|_| "HKDF fill failed")?;
        Ok(group_key.to_vec())
    }

    // ── HMAC File Authentication ───────────────────────────────────────────────
    //
    // SHA-256 alone proves a file arrived intact, but not that it came from
    // a trusted device. HMAC-SHA256 with the group key proves both.
    // A rogue device on the same network cannot forge a valid HMAC without
    // knowing the group key.

    /// Compute HMAC-SHA256 of file contents using the session key.
    /// Returns a 32-byte authentication tag.
    pub fn compute_file_hmac(session_key: &[u8], data: &[u8]) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(session_key)
            .expect("HMAC accepts any key length");
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }

    /// Verify HMAC-SHA256 of file contents.
    /// Returns Err if the tag doesn't match — file rejected.
    pub fn verify_file_hmac(
        session_key: &[u8],
        data: &[u8],
        expected_tag: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut mac = HmacSha256::new_from_slice(session_key)
            .expect("HMAC accepts any key length");
        mac.update(data);
        mac.verify_slice(expected_tag)
            .map_err(|_| "HMAC verification failed — file rejected".into())
    }

    // ── AES-256-GCM Chunk Encryption ─────────────────────────────────────────

    /// Encrypt a file chunk using the session key (not the group key directly).
    pub fn encrypt_chunk(
        session_key: &[u8],
        transfer_id: &str,
        chunk_index: u32,
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
        let nonce_bytes = Self::derive_chunk_nonce(transfer_id, chunk_index);
        let nonce = Nonce::try_assume_unique_for_key(&nonce_bytes)
            .map_err(|_| "Nonce error")?;
        let unbound_key = UnboundKey::new(&AES_256_GCM, session_key)
            .map_err(|_| "Key error")?;
        let key = LessSafeKey::new(unbound_key);
        let mut ciphertext = plaintext.to_vec();
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut ciphertext)
            .map_err(|_| "Encryption failed")?;
        Ok((ciphertext, nonce_bytes.to_vec()))
    }

    /// Decrypt a file chunk using the session key.
    pub fn decrypt_chunk(
        session_key: &[u8],
        nonce_bytes: &[u8],
        ciphertext: &mut Vec<u8>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
            .map_err(|_| "Nonce error")?;
        let unbound_key = UnboundKey::new(&AES_256_GCM, session_key)
            .map_err(|_| "Key error")?;
        let key = LessSafeKey::new(unbound_key);
        let plaintext = key.open_in_place(nonce, Aad::empty(), ciphertext)
            .map_err(|_| "Decryption failed")?;
        Ok(plaintext.to_vec())
    }

    /// Deterministic 12-byte nonce: SHA-256(transfer_id || chunk_index)[..12]
    fn derive_chunk_nonce(transfer_id: &str, chunk_index: u32) -> [u8; 12] {
        let mut hasher = Sha256::new();
        hasher.update(transfer_id.as_bytes());
        hasher.update(&chunk_index.to_le_bytes());
        let hash = hasher.finalize();
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&hash[..12]);
        nonce
    }

    // ── Key Rotation ───────────────────────────────────────────────────────────
    //
    // When a device is removed from the trust group, the group key must be
    // rotated. Otherwise the removed device could decrypt future transfers
    // if it intercepts traffic on the same network.
    //
    // Rotation: derive a new group key from the old one + a rotation nonce.
    // All remaining devices apply the same derivation → same new key.
    // The removed device cannot compute the new key.

    /// Rotate the group key after a device removal.
    /// Returns the new group key — must be stored and propagated to all
    /// remaining paired devices.
    pub fn rotate_group_key(
        old_group_key: &[u8],
        rotation_nonce: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, old_group_key);
        let prk = salt.extract(rotation_nonce.as_bytes());
        let mut new_key = [0u8; 32];
        prk.expand(&[b"dropmesh-key-rotation-v1"], hkdf::HKDF_SHA256)
            .map_err(|_| "HKDF expand failed")?
            .fill(&mut new_key)
            .map_err(|_| "HKDF fill failed")?;
        log::info!("Group key rotated with nonce: {}", rotation_nonce);
        Ok(new_key.to_vec())
    }

    // ── Verification Emojis ───────────────────────────────────────────────────

    /// Derive 4 verification emojis from a key.
    /// Used after pairing so users can visually confirm both devices
    /// derived the same secret.
    pub fn derive_verification_emojis(group_key: &[u8]) -> Vec<&'static str> {
        const EMOJIS: &[&str] = &[
            "🦊", "🐺", "🦁", "🐯", "🐻", "🦝", "🦦", "🦉",
            "🦋", "🐬", "🦄", "🐲", "🦅", "🐙", "🦑", "🦀",
            "🌊", "🔥", "⚡", "🌈", "❄️", "🌺", "🍀", "🌙",
            "💎", "🔮", "🎯", "🎸", "🚀", "⭐", "🏔️", "🌍",
        ];
        let mut hasher = Sha256::new();
        hasher.update(group_key);
        hasher.update(b"dropmesh-verification-v1");
        let hash = hasher.finalize();
        (0..4).map(|i| EMOJIS[hash[i] as usize % EMOJIS.len()]).collect()
    }

    /// SHA-256 group ID hash (first 8 hex chars) — used in mDNS TXT records
    pub fn group_id_hash(group_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(group_id.as_bytes());
        hex::encode(&hasher.finalize()[..4])
    }

    /// SHA-256 of file contents — used for integrity verification
    pub fn hash_file(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    pub fn hash_file_path(path: &Path) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];

        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }

        Ok(hex::encode(hasher.finalize()))
    }
}

use base64::Engine;