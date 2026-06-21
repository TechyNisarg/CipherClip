use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use aes_gcm_siv::Aes256GcmSiv;
use aes_gcm_siv::Nonce as LegacyNonce;
use keyring::Entry;
use std::sync::RwLock;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Sha256, Digest};

type HmacSha256 = Hmac<Sha256>;

const KEYRING_SERVICE: &str = "cipherclip";
const KEYRING_USER: &str = "local_device_key";

pub struct CryptoState {
    cipher: RwLock<XChaCha20Poly1305>,
    legacy_cipher: RwLock<Aes256GcmSiv>,
    raw_key: RwLock<Vec<u8>>,
}

impl CryptoState {
    pub fn new(app_dir: &std::path::PathBuf) -> Result<Self, String> {
        // We will try keyring first, but fallback to a local file to guarantee persistence
        // across app restarts in development and on platforms where keyring is flaky.
        let key_file = app_dir.join(".local_device_key");
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;

        let key_bytes = match entry.get_password() {
            Ok(hex_key) => hex::decode(hex_key).map_err(|e| e.to_string())?,
            Err(_) => {
                // If keyring fails, try reading from file
                if key_file.exists() {
                    let hex_key = std::fs::read_to_string(&key_file).map_err(|e| e.to_string())?;
                    hex::decode(hex_key.trim()).map_err(|e| e.to_string())?
                } else {
                    // Generate a new 32-byte key
                    let mut key = [0u8; 32];
                    OsRng.fill_bytes(&mut key);
                    let hex_key = hex::encode(key);

                    // Try to save to keyring, but always save to file as fallback
                    let _ = entry.set_password(&hex_key);
                    let _ = std::fs::write(&key_file, &hex_key);

                    key.to_vec()
                }
            }
        };

        if key_bytes.len() != 32 {
            return Err("Invalid key length stored in keyring".to_string());
        }

        let cipher = XChaCha20Poly1305::new_from_slice(&key_bytes).map_err(|e| e.to_string())?;
        let legacy_cipher = Aes256GcmSiv::new_from_slice(&key_bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            cipher: RwLock::new(cipher),
            legacy_cipher: RwLock::new(legacy_cipher),
            raw_key: RwLock::new(key_bytes.clone()),
        })
    }

    pub fn get_sync_key_hash(&self) -> String {
        let key = self.raw_key.read().unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&*key);
        hex::encode(hasher.finalize())
    }

    pub fn generate_sync_state_mac(&self, payload: &str) -> String {
        let sync_key = self.raw_key.read().unwrap();
        // 1. Derive Discovery Key via HKDF-SHA256
        let hkdf = Hkdf::<Sha256>::new(Some(b"cipherclip_discovery"), &*sync_key);
        let mut discovery_key = [0u8; 32];
        hkdf.expand(b"udp_broadcast", &mut discovery_key).expect("HKDF expansion failed");

        // 2. Generate HMAC of the payload
        let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(&discovery_key).expect("HMAC can take key of any size");
        mac.update(payload.as_bytes());
        
        // 3. Output as Hex
        hex::encode(mac.finalize().into_bytes())
    }

    pub fn verify_sync_state_mac(&self, payload: &str, provided_mac_hex: &str) -> bool {
        if let Ok(provided_mac) = hex::decode(provided_mac_hex) {
            let sync_key = self.raw_key.read().unwrap();
            let hkdf = Hkdf::<Sha256>::new(Some(b"cipherclip_discovery"), &*sync_key);
            let mut discovery_key = [0u8; 32];
            if hkdf.expand(b"udp_broadcast", &mut discovery_key).is_err() {
                return false;
            }
            if let Ok(mut mac) = <HmacSha256 as hmac::Mac>::new_from_slice(&discovery_key) {
                mac.update(payload.as_bytes());
                return mac.verify_slice(&provided_mac).is_ok();
            }
        }
        false
    }

    pub fn get_key(&self) -> Result<Vec<u8>, String> {
        Ok(self.raw_key.read().unwrap().clone())
    }

    pub fn get_key_hex(app_dir: &std::path::PathBuf) -> Result<String, String> {
        let key_file = app_dir.join(".local_device_key");
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;

        match entry.get_password() {
            Ok(hex_key) => Ok(hex_key),
            Err(_) => {
                if key_file.exists() {
                    let hex_key = std::fs::read_to_string(&key_file).map_err(|e| e.to_string())?;
                    Ok(hex_key.trim().to_string())
                } else {
                    Err("Key not found".to_string())
                }
            }
        }
    }

    pub fn set_key_hex(&self, app_dir: &std::path::PathBuf, hex_key: &str) -> Result<(), String> {
        let key_bytes = hex::decode(hex_key).map_err(|e| e.to_string())?;
        if key_bytes.len() != 32 {
            return Err("Invalid key length".to_string());
        }

        let new_cipher =
            XChaCha20Poly1305::new_from_slice(&key_bytes).map_err(|e| e.to_string())?;
        let new_legacy_cipher = Aes256GcmSiv::new_from_slice(&key_bytes).map_err(|e| e.to_string())?;

        let key_file = app_dir.join(".local_device_key");
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
        let _ = entry.set_password(hex_key);
        let _ = std::fs::write(&key_file, hex_key);

        let mut cipher_lock = self.cipher.write().unwrap();
        *cipher_lock = new_cipher;

        let mut legacy_lock = self.legacy_cipher.write().unwrap();
        *legacy_lock = new_legacy_cipher;

        let mut raw_key_lock = self.raw_key.write().unwrap();
        *raw_key_lock = key_bytes;

        Ok(())
    }

    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce_bytes = [0u8; 24];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        let mut encrypted_data = self
            .cipher
            .read()
            .unwrap()
            .encrypt(nonce, data)
            .map_err(|e| e.to_string())?;

        // Prepend nonce to the encrypted payload
        let mut result = nonce_bytes.to_vec();
        result.append(&mut encrypted_data);
        Ok(result)
    }

    pub fn decrypt(&self, encrypted_payload: &[u8]) -> Result<Vec<u8>, String> {
        if encrypted_payload.len() < 24 {
            // Might be a legacy AesGcmSiv payload which has a 12-byte nonce
            if encrypted_payload.len() >= 12 {
                let nonce = LegacyNonce::from_slice(&encrypted_payload[..12]);
                let ciphertext = &encrypted_payload[12..];
                if let Ok(decrypted) = self.legacy_cipher.read().unwrap().decrypt(nonce, ciphertext) {
                    return Ok(decrypted);
                }
            }
            return Err("Payload too short to contain nonce".to_string());
        }

        let nonce = XNonce::from_slice(&encrypted_payload[..24]);
        let ciphertext = &encrypted_payload[24..];

        // Try primary cipher first
        match self.cipher.read().unwrap().decrypt(nonce, ciphertext) {
            Ok(decrypted) => Ok(decrypted),
            Err(e) => {
                // Fallback to legacy cipher if the payload is 24+ bytes but was actually encrypted with legacy cipher
                // (Though legacy nonce is 12 bytes, if the payload length >= 12 it's possible it's a legacy payload)
                if encrypted_payload.len() >= 12 {
                    let legacy_nonce = LegacyNonce::from_slice(&encrypted_payload[..12]);
                    let legacy_ciphertext = &encrypted_payload[12..];
                    if let Ok(decrypted) = self.legacy_cipher.read().unwrap().decrypt(legacy_nonce, legacy_ciphertext) {
                        return Ok(decrypted);
                    }
                }
                Err(e.to_string())
            }
        }
    }
}
