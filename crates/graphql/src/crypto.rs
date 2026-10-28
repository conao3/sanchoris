use aes_gcm::aead::{Aead, Generate, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use uuid::Uuid;

use crate::ports::{Encrypted, Encryptor, PortError};

pub const LOCAL_KEY_ID: &str = "local-aes-256-gcm";
const NONCE_LEN: usize = 12;

pub struct AesGcmEncryptor {
    cipher: Aes256Gcm,
}

impl AesGcmEncryptor {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key)),
        }
    }

    pub fn from_base64(encoded: &str) -> Result<Self, String> {
        let bytes = STANDARD
            .decode(encoded.trim())
            .map_err(|e| format!("not base64: {e}"))?;
        let key: [u8; 32] = bytes
            .try_into()
            .map_err(|bytes: Vec<u8>| format!("must decode to 32 bytes, got {}", bytes.len()))?;
        Ok(Self::new(&key))
    }

    pub fn decrypt(&self, tenant_id: Uuid, data: &[u8]) -> Result<Vec<u8>, PortError> {
        if data.len() < NONCE_LEN {
            return Err(PortError::Failed(
                "the ciphertext is shorter than its nonce".into(),
            ));
        }
        let (nonce, ciphertext) = data.split_at(NONCE_LEN);
        self.cipher
            .decrypt(
                &Nonce::from(
                    <[u8; NONCE_LEN]>::try_from(nonce).expect("the nonce has NONCE_LEN bytes"),
                ),
                Payload {
                    msg: ciphertext,
                    aad: tenant_id.as_bytes(),
                },
            )
            .map_err(|_| PortError::Failed("the credential does not decrypt with this key".into()))
    }
}

#[async_trait]
impl Encryptor for AesGcmEncryptor {
    async fn encrypt(&self, tenant_id: Uuid, plaintext: &[u8]) -> Result<Encrypted, PortError> {
        let nonce = Nonce::generate();
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: tenant_id.as_bytes(),
                },
            )
            .map_err(|_| PortError::Failed("the credential could not be encrypted".into()))?;
        let mut data = nonce.to_vec();
        data.extend(ciphertext);
        Ok(Encrypted {
            ciphertext: data,
            key_id: LOCAL_KEY_ID.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_and_binds_the_tenant() {
        let encryptor = AesGcmEncryptor::new(&[7; 32]);
        let tenant = Uuid::new_v4();
        let encrypted = encryptor.encrypt(tenant, b"sk-ant-secret").await.unwrap();
        assert_eq!(encrypted.key_id, LOCAL_KEY_ID);
        assert_ne!(&encrypted.ciphertext[NONCE_LEN..], b"sk-ant-secret");
        assert_eq!(
            encryptor.decrypt(tenant, &encrypted.ciphertext).unwrap(),
            b"sk-ant-secret"
        );
        assert!(
            encryptor
                .decrypt(Uuid::new_v4(), &encrypted.ciphertext)
                .is_err()
        );
    }
}
