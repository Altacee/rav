use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use rand::RngCore;

/// A credential at rest: a 12-byte nonce and the AES-256-GCM ciphertext.
///
/// The key never lives beside this — it comes from `PUSH_CREDENTIAL_KEY`, a
/// cluster Secret — so a stolen data volume yields nothing on its own.
pub struct Sealed {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub fn seal(key: &[u8; 32], plaintext: &str) -> Result<Sealed, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .map_err(|_| "could not encrypt the push credential".to_string())?;
    Ok(Sealed { nonce: nonce_bytes.to_vec(), ciphertext })
}

/// Fails closed: a wrong key, a truncated nonce or a tampered ciphertext all
/// return an error rather than anything that could be mistaken for a password.
pub fn open(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<String, String> {
    if nonce.len() != 12 {
        return Err("push credential nonce is not 12 bytes".to_string());
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| "push credential did not decrypt - wrong key or tampered data".to_string())?;
    String::from_utf8(plaintext).map_err(|_| "push credential is not valid UTF-8".to_string())
}

/// Parse `PUSH_CREDENTIAL_KEY`: standard base64 of exactly 32 bytes. Anything
/// else is treated as "not configured" rather than silently padded into a key.
pub fn parse_key(encoded: &str) -> Option<[u8; 32]> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    let array: [u8; 32] = bytes.try_into().ok()?;
    Some(array)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        [7u8; 32]
    }

    #[test]
    fn a_sealed_credential_opens_with_the_same_key() {
        let sealed = seal(&key(), "app-password-123").unwrap();
        assert_eq!(open(&key(), &sealed.nonce, &sealed.ciphertext).unwrap(), "app-password-123");
    }

    #[test]
    fn the_ciphertext_does_not_contain_the_password() {
        let sealed = seal(&key(), "app-password-123").unwrap();
        let haystack = String::from_utf8_lossy(&sealed.ciphertext).to_string();
        assert!(!haystack.contains("app-password"), "got: {haystack}");
    }

    #[test]
    fn a_different_key_fails_closed() {
        let sealed = seal(&key(), "app-password-123").unwrap();
        let wrong = [9u8; 32];
        assert!(
            open(&wrong, &sealed.nonce, &sealed.ciphertext).is_err(),
            "a wrong key must fail, never return plaintext or garbage"
        );
    }

    #[test]
    fn tampering_with_the_ciphertext_fails_closed() {
        let sealed = seal(&key(), "app-password-123").unwrap();
        let mut tampered = sealed.ciphertext.clone();
        tampered[0] ^= 0xff;
        assert!(open(&key(), &sealed.nonce, &tampered).is_err());
    }

    #[test]
    fn each_seal_uses_a_fresh_nonce() {
        let a = seal(&key(), "same").unwrap();
        let b = seal(&key(), "same").unwrap();
        assert_ne!(a.nonce, b.nonce, "a reused nonce would leak equality of passwords");
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn the_key_must_be_32_bytes_of_base64() {
        assert!(parse_key("").is_none());
        assert!(parse_key("not base64!!").is_none());
        assert!(parse_key(&base64_std(&[1u8; 16])).is_none(), "16 bytes is not a 256-bit key");
        assert_eq!(parse_key(&base64_std(&[1u8; 32])), Some([1u8; 32]));
    }

    fn base64_std(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }
}
