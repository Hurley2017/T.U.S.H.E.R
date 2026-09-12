use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(String);

impl NodeId {
    pub fn from_verifying_key(vk: &VerifyingKey) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(vk.as_bytes());
        let hash = hasher.finalize();
        let encoded = data_encoding::BASE32_NOPAD.encode(&hash[..15]).to_lowercase();
        NodeId(format!("tshr_{}", encoded))
    }

    pub fn from_str_unchecked(s: &str) -> Self {
        NodeId(s.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for NodeId {
    fn from(s: String) -> Self {
        NodeId(s)
    }
}

impl From<&str> for NodeId {
    fn from(s: &str) -> Self {
        NodeId(s.to_string())
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Windows,
    Linux,
    MacOS,
    Android,
    Unknown,
}

impl Platform {
    pub fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Platform::Windows
        }
        #[cfg(target_os = "linux")]
        {
            Platform::Linux
        }
        #[cfg(target_os = "macos")]
        {
            Platform::MacOS
        }
        #[cfg(target_os = "android")]
        {
            Platform::Android
        }
        #[cfg(not(any(
            target_os = "windows",
            target_os = "linux",
            target_os = "macos",
            target_os = "android"
        )))]
        {
            Platform::Unknown
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Platform::Windows => write!(f, "windows"),
            Platform::Linux => write!(f, "linux"),
            Platform::MacOS => write!(f, "macos"),
            Platform::Android => write!(f, "android"),
            Platform::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    pub private_key_hex: String,
    pub node_name: String,
    pub created_at: i64,
}

pub struct DeviceIdentity {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    node_id: NodeId,
    node_name: String,
    platform: Platform,
    created_at: i64,
}

impl DeviceIdentity {
    pub fn generate(node_name: String) -> Self {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        let node_id = NodeId::from_verifying_key(&verifying_key);
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        Self {
            signing_key,
            verifying_key,
            node_id,
            node_name,
            platform: Platform::current(),
            created_at,
        }
    }

    pub fn load_or_create<P: AsRef<Path>>(storage_dir: P, default_name: &str) -> anyhow::Result<Self> {
        let dir = storage_dir.as_ref();
        fs::create_dir_all(dir)?;
        let key_path = dir.join("identity.json");

        if key_path.exists() {
            let data = fs::read_to_string(&key_path)?;
            let stored: StoredIdentity = serde_json::from_str(&data)?;
            let key_bytes = hex::decode(&stored.private_key_hex)?;
            let signing_key = SigningKey::try_from(key_bytes.as_slice())
                .map_err(|e| anyhow::anyhow!("Invalid private key in storage: {}", e))?;
            let verifying_key = signing_key.verifying_key();
            let node_id = NodeId::from_verifying_key(&verifying_key);

            Ok(Self {
                signing_key,
                verifying_key,
                node_id,
                node_name: stored.node_name,
                platform: Platform::current(),
                created_at: stored.created_at,
            })
        } else {
            let identity = Self::generate(default_name.to_string());
            let stored = StoredIdentity {
                private_key_hex: hex::encode(identity.signing_key.to_bytes()),
                node_name: identity.node_name.clone(),
                created_at: identity.created_at,
            };
            let json = serde_json::to_string_pretty(&stored)?;
            fs::write(&key_path, json)?;
            Ok(identity)
        }
    }

    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    pub fn node_name(&self) -> &str {
        &self.node_name
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.verifying_key.as_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), ed25519_dalek::SignatureError> {
        self.verifying_key.verify(message, signature)
    }
}
