use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use keyring::Entry;
use std::sync::RwLock;

const KEYRING_SERVICE: &str = "cipherclip";
const KEYRING_USER: &str = "local_device_key";

pub struct CryptoState {
    cipher: RwLock<XChaCha20Poly1305>,
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
        Ok(Self {
            cipher: RwLock::new(cipher),
        })
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

        let key_file = app_dir.join(".local_device_key");
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
        let _ = entry.set_password(hex_key);
        let _ = std::fs::write(&key_file, hex_key);

        let mut cipher_lock = self.cipher.write().unwrap();
        *cipher_lock = new_cipher;

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
            return Err("Payload too short to contain nonce".to_string());
        }

        let nonce = XNonce::from_slice(&encrypted_payload[..24]);
        let ciphertext = &encrypted_payload[24..];

        self.cipher
            .read()
            .unwrap()
            .decrypt(nonce, ciphertext)
            .map_err(|e| e.to_string())
    }
}
