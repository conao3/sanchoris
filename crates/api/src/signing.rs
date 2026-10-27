use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct HmacKey(Vec<u8>);

impl std::fmt::Debug for HmacKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HmacKey(..)")
    }
}

impl HmacKey {
    pub fn new(key: &[u8]) -> Result<Self, String> {
        if key.is_empty() {
            return Err("the key is empty".into());
        }
        Ok(Self(key.to_vec()))
    }

    pub fn strong(key: &[u8]) -> Result<Self, String> {
        if key.len() < 32 {
            return Err("the signing key must be at least 32 bytes".into());
        }
        Ok(Self(key.to_vec()))
    }

    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.0).expect("HMAC accepts any key length")
    }

    pub fn hex(&self, message: &[u8]) -> String {
        let mut mac = self.mac();
        mac.update(message);
        hex::encode(mac.finalize().into_bytes())
    }

    pub fn verify_hex(&self, message: &[u8], signature: &str) -> bool {
        let Ok(bytes) = hex::decode(signature) else {
            return false;
        };
        let mut mac = self.mac();
        mac.update(message);
        mac.verify_slice(&bytes).is_ok()
    }

    pub fn seal(&self, message: &str) -> String {
        format!("{message}.{}", self.hex(message.as_bytes()))
    }

    pub fn open<'a>(&self, sealed: &'a str) -> Option<&'a str> {
        let (message, signature) = sealed.rsplit_once('.')?;
        self.verify_hex(message.as_bytes(), signature)
            .then_some(message)
    }
}

#[cfg(test)]
mod tests {
    use super::HmacKey;

    #[test]
    fn sealed_messages_reject_tampering_and_other_keys() {
        let key = HmacKey::strong(&[1u8; 32]).unwrap();
        let sealed = key.seal("user.123");
        assert_eq!(key.open(&sealed), Some("user.123"));
        assert_eq!(key.open(&sealed.replace("123", "124")), None);
        assert_eq!(HmacKey::strong(&[2u8; 32]).unwrap().open(&sealed), None);
        assert_eq!(key.open("no-signature"), None);
        assert!(HmacKey::strong(b"short").is_err());
        assert!(HmacKey::new(b"").is_err());
    }

    #[test]
    fn hex_signatures_match_the_github_test_vector() {
        let key = HmacKey::new(b"It's a Secret to Everybody").unwrap();
        assert_eq!(
            key.hex(b"Hello, World!"),
            "757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17"
        );
        assert!(key.verify_hex(
            b"Hello, World!",
            "757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17"
        ));
        assert!(!key.verify_hex(b"Hello, World!", "00"));
    }
}
