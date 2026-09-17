use serde::{Deserialize, Serialize};

/// The type of device running DropMesh
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    Laptop,
    Phone,
    Tablet,
    Desktop,
}

impl DeviceType {
    /// Auto-detect the current device type based on platform
    pub fn detect() -> Self {
        // TODO: Use platform-specific detection
        // For now, default to desktop on desktop OSes
        #[cfg(target_os = "ios")]
        return DeviceType::Phone;
        #[cfg(target_os = "android")]
        return DeviceType::Phone;
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        DeviceType::Laptop
    }
}

impl std::fmt::Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceType::Laptop => write!(f, "laptop"),
            DeviceType::Phone => write!(f, "phone"),
            DeviceType::Tablet => write!(f, "tablet"),
            DeviceType::Desktop => write!(f, "desktop"),
        }
    }
}

/// This device's own identity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfDevice {
    pub id: String,
    pub name: String,
    pub device_type: DeviceType,
    pub public_key: String, // base64-encoded
    pub created_at: String,
}

/// A paired device in the trust group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub device_id: String,
    pub name: String,
    pub device_type: DeviceType,
    pub public_key: String,
    pub group_id: String,
    pub paired_at: String,
    pub last_seen_at: Option<String>,
    pub is_online: bool,
}

/// Trust group containing this device and its paired devices
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustGroup {
    pub group_id: String,
    pub created_at: String,
}

/// Pairing payload exchanged between devices
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingPayload {
    pub device_id: String,
    pub public_key: String,
    pub group_id: String,
    pub nonce: String,
    pub expires: String,
    pub pairing_code: String, // 6-digit numeric code
    pub verification_emojis: String,
}

/// Names used for auto-generating device names
const ADJECTIVES: &[&str] = &[
    "Blue", "Red", "Green", "Silver", "Golden",
    "Swift", "Calm", "Bold", "Bright", "Dark",
    "Warm", "Cool", "Wild", "Quiet", "Lucky",
];

const ANIMALS: &[&str] = &[
    "Penguin", "Fox", "Wolf", "Owl", "Bear",
    "Hawk", "Lynx", "Otter", "Raven", "Hare",
    "Falcon", "Tiger", "Panda", "Eagle", "Whale",
];

/// Generate a random friendly device name like "Blue Penguin"
pub fn generate_device_name() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let adj = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];
    let animal = ANIMALS[rng.gen_range(0..ANIMALS.len())];
    format!("{} {}", adj, animal)
}
