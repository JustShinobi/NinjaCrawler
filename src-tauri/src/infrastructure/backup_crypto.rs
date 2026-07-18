//! Password-based encryption for workspace-backup secret material.
//!
//! DPAPI (see [`session_secret_store`]) is machine/user bound and therefore not
//! portable inside a backup that may be restored on another machine. When the
//! user opts to include account secrets in a backup we re-encrypt the material
//! with a password: Argon2id derives a 256-bit key, AES-256-GCM seals the
//! plaintext. Nothing is ever written to disk in the clear.
//!
//! Serialized envelope (little-endian):
//!   magic      "NCBKSEC\0"  (8 bytes)
//!   version    u8           (currently 1)
//!   salt_len   u8
//!   salt       [salt_len]   (Argon2 salt, 16 bytes)
//!   m_cost     u32          (Argon2 memory cost, KiB)
//!   t_cost     u32          (Argon2 iterations)
//!   p_cost     u32          (Argon2 parallelism)
//!   nonce      [12]         (AES-GCM nonce)
//!   ciphertext [..]         (AES-256-GCM sealed plaintext + tag)

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;

const MAGIC: &[u8; 8] = b"NCBKSEC\0";
const FORMAT_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

// OWASP-recommended baseline for Argon2id (memory-hard). 19 MiB / 2 passes / 1
// lane. Stored in the envelope so decryption stays correct even if the defaults
// change in a future release.
const DEFAULT_M_COST_KIB: u32 = 19 * 1024;
const DEFAULT_T_COST: u32 = 2;
const DEFAULT_P_COST: u32 = 1;

fn derive_key(password: &str, salt: &[u8], m_cost: u32, t_cost: u32, p_cost: u32) -> Result<[u8; KEY_LEN], String> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN))
        .map_err(|error| format!("Invalid Argon2 parameters: {error}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|error| format!("Key derivation failed: {error}"))?;
    Ok(key)
}

/// Seals `plaintext` under `password`. Returns the self-describing envelope.
pub fn encrypt(plaintext: &[u8], password: &str) -> Result<Vec<u8>, String> {
    if password.is_empty() {
        return Err("A password is required to encrypt backup secrets.".to_string());
    }

    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    let mut rng = rand::thread_rng();
    rng.fill_bytes(&mut salt);
    rng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(
        password,
        &salt,
        DEFAULT_M_COST_KIB,
        DEFAULT_T_COST,
        DEFAULT_P_COST,
    )?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|error| format!("Cipher init failed: {error}"))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|_| "Encryption failed.".to_string())?;

    let mut out = Vec::with_capacity(
        MAGIC.len() + 2 + SALT_LEN + 12 + NONCE_LEN + ciphertext.len(),
    );
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.push(SALT_LEN as u8);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&DEFAULT_M_COST_KIB.to_le_bytes());
    out.extend_from_slice(&DEFAULT_T_COST.to_le_bytes());
    out.extend_from_slice(&DEFAULT_P_COST.to_le_bytes());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Opens an envelope produced by [`encrypt`]. A wrong password surfaces as an
/// error (the GCM tag fails to verify) rather than returning garbage.
pub fn decrypt(blob: &[u8], password: &str) -> Result<Vec<u8>, String> {
    if password.is_empty() {
        return Err("A password is required to decrypt backup secrets.".to_string());
    }

    let header_min = MAGIC.len() + 2;
    if blob.len() < header_min {
        return Err("Backup secret payload is truncated.".to_string());
    }
    if &blob[..MAGIC.len()] != MAGIC {
        return Err("Backup secret payload has an unexpected format.".to_string());
    }

    let mut offset = MAGIC.len();
    let version = blob[offset];
    offset += 1;
    if version != FORMAT_VERSION {
        return Err(format!("Unsupported backup secret version {version}."));
    }
    let salt_len = blob[offset] as usize;
    offset += 1;

    // salt + 3 u32 params + nonce
    let needed = salt_len + 12 + NONCE_LEN;
    if blob.len() < offset + needed {
        return Err("Backup secret payload is truncated.".to_string());
    }

    let salt = &blob[offset..offset + salt_len];
    offset += salt_len;
    let m_cost = u32::from_le_bytes(blob[offset..offset + 4].try_into().unwrap());
    offset += 4;
    let t_cost = u32::from_le_bytes(blob[offset..offset + 4].try_into().unwrap());
    offset += 4;
    let p_cost = u32::from_le_bytes(blob[offset..offset + 4].try_into().unwrap());
    offset += 4;
    let nonce_bytes = &blob[offset..offset + NONCE_LEN];
    offset += NONCE_LEN;
    let ciphertext = &blob[offset..];

    let key = derive_key(password, salt, m_cost, t_cost, p_cost)?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|error| format!("Cipher init failed: {error}"))?;
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| "Wrong password or corrupted backup secrets.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_correct_password() {
        let plaintext = br#"{"account-1":"{\"cookie\":\"value\"}"}"#;
        let blob = encrypt(plaintext, "correct horse battery staple").expect("encrypt");
        assert_ne!(&blob[..], &plaintext[..], "payload must not be stored in the clear");
        let restored = decrypt(&blob, "correct horse battery staple").expect("decrypt");
        assert_eq!(restored, plaintext);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let plaintext = b"top secret session material";
        let blob = encrypt(plaintext, "right-password").expect("encrypt");
        let error = decrypt(&blob, "wrong-password").expect_err("must reject wrong password");
        assert!(error.to_lowercase().contains("wrong password"));
    }

    #[test]
    fn distinct_salts_and_nonces_across_calls() {
        let plaintext = b"same input";
        let a = encrypt(plaintext, "pw").expect("encrypt a");
        let b = encrypt(plaintext, "pw").expect("encrypt b");
        // Random salt + nonce make identical plaintext produce distinct blobs.
        assert_ne!(a, b);
    }

    #[test]
    fn empty_password_is_rejected() {
        assert!(encrypt(b"x", "").is_err());
        assert!(decrypt(&[0u8; 40], "").is_err());
    }

    #[test]
    fn corrupted_ciphertext_is_rejected() {
        let mut blob = encrypt(b"payload", "pw").expect("encrypt");
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(decrypt(&blob, "pw").is_err());
    }
}
