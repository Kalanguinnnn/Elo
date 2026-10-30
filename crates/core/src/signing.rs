//! Checkpoint authenticity (audit finding 3).
//!
//! Every device holds an Ed25519 key in the OS credential vault and signs the
//! fields that determine what an apply would write: id, device, branch, HEAD,
//! staging commit, tree, lineage parents and time. Other devices pin the first
//! key seen for a device id and only auto-apply checkpoints whose signature
//! verifies against a key the user explicitly approved.

use crate::checkpoint::CheckpointMeta;
use crate::credentials::SecretStore;
use crate::error::{Error, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

pub const KEY_ACCOUNT: &str = "device-signing-key";

pub struct DeviceKey {
    key: SigningKey,
}

impl DeviceKey {
    /// Load this device's key from the vault, creating it on first use.
    pub fn load_or_create(store: &dyn SecretStore) -> Result<DeviceKey> {
        if let Some(hex_key) = store.get(KEY_ACCOUNT)? {
            let bytes: [u8; 32] = hex::decode(hex_key.trim())
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| Error::Credentials("chave de assinatura corrompida no cofre".into()))?;
            return Ok(DeviceKey { key: SigningKey::from_bytes(&bytes) });
        }
        let seed = crate::util::random_bytes::<32>();
        store.set(KEY_ACCOUNT, &hex::encode(seed))?;
        Ok(DeviceKey { key: SigningKey::from_bytes(&seed) })
    }

    pub fn public_key(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }

    pub fn sign(&self, payload: &[u8]) -> String {
        hex::encode(self.key.sign(payload).to_bytes())
    }
}

/// Canonical byte string covered by the signature. `tree` is taken from the
/// actual commit when verifying, so a signature cannot be reused for other
/// content.
pub fn payload(meta: &CheckpointMeta, tree: &str) -> Vec<u8> {
    format!(
        "elo-checkpoint-v1\nid={}\ndevice={}\nbranch={}\nhead={}\nindex={}\ntree={}\nparents={}\ncreated={}\n",
        meta.id,
        meta.device_id,
        meta.branch.as_deref().unwrap_or(""),
        meta.head.as_deref().unwrap_or(""),
        meta.index_commit,
        tree,
        meta.parents.join(" "),
        meta.created_ms
    )
    .into_bytes()
}

/// Whether `meta` carries a valid signature by its embedded key over `tree`.
pub fn verify(meta: &CheckpointMeta, tree: &str) -> bool {
    let (Some(pk), Some(sig)) = (&meta.public_key, &meta.signature) else { return false };
    let Some(pk) = hex::decode(pk).ok().and_then(|v| <[u8; 32]>::try_from(v).ok()) else { return false };
    let Some(sig) = hex::decode(sig).ok().and_then(|v| <[u8; 64]>::try_from(v).ok()) else { return false };
    let Ok(vk) = VerifyingKey::from_bytes(&pk) else { return false };
    vk.verify_strict(&payload(meta, tree), &Signature::from_bytes(&sig)).is_ok()
}

/// Short, human-comparable fingerprint shown on both machines when pairing.
pub fn fingerprint(public_key_hex: &str) -> String {
    let digest = crate::util::sha256_hex(public_key_hex.as_bytes());
    digest.as_bytes()[..20].chunks(4).map(|c| String::from_utf8_lossy(c).to_uppercase()).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::{new_id, Trigger};
    use crate::credentials::MemoryStore;

    fn meta() -> CheckpointMeta {
        CheckpointMeta {
            id: new_id(1_700_000_000_000),
            device_id: uuid::Uuid::new_v4().to_string(),
            device_name: "pc".into(),
            branch: Some("refs/heads/main".into()),
            head: Some("a".repeat(40)),
            index_commit: "b".repeat(40),
            created_ms: 1_700_000_000_000,
            trigger: Trigger::Timer,
            parents: vec![],
            summary: String::new(),
            public_key: None,
            signature: None,
        }
    }

    #[test]
    fn sign_verify_and_tamper() {
        let store = MemoryStore::default();
        let k = DeviceKey::load_or_create(&store).unwrap();
        assert_eq!(DeviceKey::load_or_create(&store).unwrap().public_key(), k.public_key(), "persisted");
        let mut m = meta();
        let tree = "c".repeat(40);
        m.public_key = Some(k.public_key());
        m.signature = Some(k.sign(&payload(&m, &tree)));
        assert!(verify(&m, &tree));
        assert!(!verify(&m, &"d".repeat(40)), "different content");
        let mut forged = m.clone();
        forged.parents = vec![new_id(1)];
        assert!(!verify(&forged, &tree), "different lineage");
        let other = DeviceKey::load_or_create(&MemoryStore::default()).unwrap();
        let mut stolen = m.clone();
        stolen.public_key = Some(other.public_key());
        assert!(!verify(&stolen, &tree), "signature does not match a swapped key");
        assert_eq!(fingerprint(&k.public_key()).len(), 24);
    }
}
