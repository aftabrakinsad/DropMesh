/// DropMesh Pairing Handshake
///
/// Solves the "Unknown device" problem: two devices must exchange their
/// REAL identities (device_id, name, type, public_key) over the network
/// before they can be considered paired.
///
/// Flow:
///   1. Device A taps "+ Add device" → generates PIN, shows it, waits
///   2. Device B sees A in the Nearby list (via mDNS), taps it, enters the PIN
///   3. Device B connects to A's QUIC endpoint (host:port from mDNS)
///   4. B sends PairRequest { device_id, name, type, public_key, pin }
///   5. A validates the PIN, stores B as a paired device, replies PairResponse
///   6. B stores A as a paired device
///   7. Both derive the same group key from the PIN → same emojis
///
/// Now both sides have the OTHER device's real ID, so mDNS discovery
/// will correctly match them and mark them online.

use serde::{Deserialize, Serialize};

/// Pairing messages exchanged over QUIC
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PairMessage {
    /// Device B → Device A: "I want to pair, here's my identity and the PIN"
    PairRequest {
        device_id: String,
        name: String,
        device_type: String,
        public_key: String,   // base64
        cert_fingerprint: String,
        pin: String,          // the 6-digit code shown on Device A
    },
    /// Device A → Device B: "PIN accepted, here's my identity"
    PairResponse {
        device_id: String,
        name: String,
        device_type: String,
        public_key: String,   // base64
        cert_fingerprint: String,
        group_id: String,
        verification_emojis: String,
    },
    /// Device A → Device B: "PIN rejected"
    PairRejected {
        reason: String,
    },
}