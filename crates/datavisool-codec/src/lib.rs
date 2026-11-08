/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::AeadCore;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use flate2::read::GzDecoder;
use datavisool_project_format::{
    EncryptedEnvelopeV1, ENCRYPTED_ENVELOPE_ALG_A256_GCM, ENCRYPTED_ENVELOPE_KDF_ARGON2_ID,
    ENCRYPTED_FIELD_PREFIX_V1,
};
use std::io::Read;

pub const PROJECT_COMPRESSION_PREFIX: &str = "DVZ1:gzip:";

pub const ARGON2ID_DEFAULT_KEY_LEN: usize = 32;
pub const ARGON2ID_DEFAULT_M_COST_KIB: u32 = 19_456;
pub const ARGON2ID_DEFAULT_T_COST: u32 = 2;
pub const ARGON2ID_DEFAULT_P_COST: u32 = 1;
pub const ENCRYPTED_ENVELOPE_SALT_LEN: usize = 16;
pub const AES_256_GCM_NONCE_LEN: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("invalid project compression prefix")]
    InvalidPrefix,
    #[error("base64 decode failed")]
    Base64,
    #[error("gzip decode failed")]
    Gzip,
    #[error("utf8 decode failed")]
    Utf8,
    #[error("argon2 parameter invalid")]
    Argon2Params,
    #[error("argon2 key derivation failed")]
    Argon2KeyDerive,
    #[error("invalid encrypted field prefix")]
    InvalidEncryptedPrefix,
    #[error("invalid encrypted envelope payload")]
    InvalidEncryptedEnvelopePayload,
    #[error("invalid encrypted envelope json")]
    InvalidEncryptedEnvelopeJson,
    #[error("invalid encrypted envelope algorithm")]
    InvalidEncryptedEnvelopeAlgorithm,
    #[error("invalid encrypted envelope kdf")]
    InvalidEncryptedEnvelopeKdf,
    #[error("invalid encrypted envelope nonce")]
    InvalidEncryptedEnvelopeNonce,
    #[error("aes key init failed")]
    AesKeyInit,
    #[error("aes encrypt failed")]
    AesEncrypt,
    #[error("aes decrypt failed")]
    AesDecrypt,
    #[error("encrypted envelope json serialize failed")]
    EncryptedEnvelopeSerialize,
}

fn compressed_gzip_payload(input: &str) -> Option<&str> {
    input.strip_prefix(PROJECT_COMPRESSION_PREFIX)
}

pub fn decompress_project_gzip(input: &str) -> Result<String, CodecError> {
    let s = input.trim();
    let payload = compressed_gzip_payload(s)
        .ok_or(CodecError::InvalidPrefix)?;
    let bytes = STANDARD.decode(payload).map_err(|_| CodecError::Base64)?;
    let mut gz = GzDecoder::new(&bytes[..]);
    let mut out = Vec::new();
    gz.read_to_end(&mut out).map_err(|_| CodecError::Gzip)?;
    String::from_utf8(out).map_err(|_| CodecError::Utf8)
}

pub fn maybe_decompress_project_json(input: &str) -> Result<String, CodecError> {
    let s = input.trim();
    if compressed_gzip_payload(s).is_some() {
        decompress_project_gzip(s)
    } else {
        Ok(s.to_string())
    }
}

pub fn derive_argon2id_key(
    passphrase: &str,
    salt: &[u8],
    key_len: usize,
) -> Result<Vec<u8>, CodecError> {
    let params = Params::new(
        ARGON2ID_DEFAULT_M_COST_KIB,
        ARGON2ID_DEFAULT_T_COST,
        ARGON2ID_DEFAULT_P_COST,
        Some(key_len),
    )
    .map_err(|_| CodecError::Argon2Params)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = vec![0_u8; key_len];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|_| CodecError::Argon2KeyDerive)?;
    Ok(key)
}

pub fn encrypt_with_passphrase_v1(
    plaintext: &[u8],
    passphrase: &str,
) -> Result<String, CodecError> {
    let envelope = encrypt_envelope_v1(plaintext, passphrase)?;
    let envelope_json =
        serde_json::to_vec(&envelope).map_err(|_| CodecError::EncryptedEnvelopeSerialize)?;
    Ok(format!(
        "{}{}",
        ENCRYPTED_FIELD_PREFIX_V1,
        URL_SAFE_NO_PAD.encode(envelope_json)
    ))
}

pub fn decrypt_with_passphrase_v1(input: &str, passphrase: &str) -> Result<Vec<u8>, CodecError> {
    let payload = input
        .trim()
        .strip_prefix(ENCRYPTED_FIELD_PREFIX_V1)
        .ok_or(CodecError::InvalidEncryptedPrefix)?;
    let envelope_bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| CodecError::InvalidEncryptedEnvelopePayload)?;
    let envelope: EncryptedEnvelopeV1 = serde_json::from_slice(&envelope_bytes)
        .map_err(|_| CodecError::InvalidEncryptedEnvelopeJson)?;
    decrypt_envelope_v1(&envelope, passphrase)
}

pub fn encrypt_envelope_v1(
    plaintext: &[u8],
    passphrase: &str,
) -> Result<EncryptedEnvelopeV1, CodecError> {
    let mut salt = [0_u8; ENCRYPTED_ENVELOPE_SALT_LEN];
    OsRng.fill_bytes(&mut salt);

    let key = derive_argon2id_key(passphrase, &salt, ARGON2ID_DEFAULT_KEY_LEN)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| CodecError::AesKeyInit)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| CodecError::AesEncrypt)?;

    Ok(EncryptedEnvelopeV1 {
        alg: ENCRYPTED_ENVELOPE_ALG_A256_GCM.to_string(),
        kdf: ENCRYPTED_ENVELOPE_KDF_ARGON2_ID.to_string(),
        salt: STANDARD.encode(salt),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
}

pub fn decrypt_envelope_v1(
    envelope: &EncryptedEnvelopeV1,
    passphrase: &str,
) -> Result<Vec<u8>, CodecError> {
    if envelope.alg.trim() != ENCRYPTED_ENVELOPE_ALG_A256_GCM {
        return Err(CodecError::InvalidEncryptedEnvelopeAlgorithm);
    }
    if envelope.kdf.trim() != ENCRYPTED_ENVELOPE_KDF_ARGON2_ID {
        return Err(CodecError::InvalidEncryptedEnvelopeKdf);
    }

    let salt = STANDARD
        .decode(envelope.salt.trim())
        .map_err(|_| CodecError::InvalidEncryptedEnvelopePayload)?;
    let nonce_bytes = STANDARD
        .decode(envelope.nonce.trim())
        .map_err(|_| CodecError::InvalidEncryptedEnvelopePayload)?;
    if nonce_bytes.len() != AES_256_GCM_NONCE_LEN {
        return Err(CodecError::InvalidEncryptedEnvelopeNonce);
    }
    let ciphertext = STANDARD
        .decode(envelope.ciphertext.trim())
        .map_err(|_| CodecError::InvalidEncryptedEnvelopePayload)?;

    let key = derive_argon2id_key(passphrase, &salt, ARGON2ID_DEFAULT_KEY_LEN)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| CodecError::AesKeyInit)?;

    cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
        .map_err(|_| CodecError::AesDecrypt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn decompresses_current_project_data() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(br#"{"project":"example"}"#).unwrap();
        let encoded = STANDARD.encode(encoder.finish().unwrap());
        for prefix in ["DVZ1:gzip:"] {
            assert_eq!(maybe_decompress_project_json(&format!("{prefix}{encoded}")).unwrap(), r#"{"project":"example"}"#);
        }
    }
    use datavisool_project_format::{CommonApiRef, CommonDatabaseRef};

    #[test]
    fn derive_argon2id_key_returns_requested_length() {
        let key = derive_argon2id_key(
            "export-passphrase",
            b"runtime-salt-001",
            ARGON2ID_DEFAULT_KEY_LEN,
        )
        .expect("derive argon2id key failed");

        assert_eq!(key.len(), ARGON2ID_DEFAULT_KEY_LEN);
    }

    #[test]
    fn derive_argon2id_key_is_deterministic_for_same_inputs() {
        let first = derive_argon2id_key(
            "export-passphrase",
            b"runtime-salt-001",
            ARGON2ID_DEFAULT_KEY_LEN,
        )
        .expect("first derive argon2id key failed");
        let second = derive_argon2id_key(
            "export-passphrase",
            b"runtime-salt-001",
            ARGON2ID_DEFAULT_KEY_LEN,
        )
        .expect("second derive argon2id key failed");
        let different_salt = derive_argon2id_key(
            "export-passphrase",
            b"runtime-salt-002",
            ARGON2ID_DEFAULT_KEY_LEN,
        )
        .expect("derive argon2id key with different salt failed");

        assert_eq!(first, second);
        assert_ne!(first, different_salt);
    }

    #[test]
    fn encrypt_with_passphrase_v1_uses_expected_prefix_and_round_trip_succeeds() {
        let plaintext = br#"{"commonApis":[{"originId":"1"}]}"#;

        let encrypted = encrypt_with_passphrase_v1(plaintext, "export-passphrase")
            .expect("encrypt with passphrase failed");
        let decrypted = decrypt_with_passphrase_v1(&encrypted, "export-passphrase")
            .expect("decrypt with passphrase failed");

        assert!(encrypted.starts_with(ENCRYPTED_FIELD_PREFIX_V1));
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decrypt_with_wrong_passphrase_fails() {
        let encrypted = encrypt_with_passphrase_v1(
            br#"{"databases":[{"originId":"2"}]}"#,
            "correct-passphrase",
        )
        .expect("encrypt with passphrase failed");

        let result = decrypt_with_passphrase_v1(&encrypted, "wrong-passphrase");

        assert!(matches!(result, Err(CodecError::AesDecrypt)));
    }

    #[test]
    fn decrypt_envelope_v1_rejects_invalid_algorithm_and_kdf() {
        let encrypted = encrypt_envelope_v1(br#"{"hello":"world"}"#, "export-passphrase")
            .expect("encrypt envelope failed");

        let invalid_alg = EncryptedEnvelopeV1 {
            alg: "ChaCha20".to_string(),
            ..encrypted.clone()
        };
        let invalid_kdf = EncryptedEnvelopeV1 {
            kdf: "PBKDF2".to_string(),
            ..encrypted
        };

        assert!(matches!(
            decrypt_envelope_v1(&invalid_alg, "export-passphrase"),
            Err(CodecError::InvalidEncryptedEnvelopeAlgorithm)
        ));
        assert!(matches!(
            decrypt_envelope_v1(&invalid_kdf, "export-passphrase"),
            Err(CodecError::InvalidEncryptedEnvelopeKdf)
        ));
    }

    #[test]
    fn encrypt_with_passphrase_v1_round_trips_common_apis_and_databases_fields() {
        let common_apis = vec![CommonApiRef {
            origin_id: "api-1".to_string(),
            ref_key: "common-api-ref-1".to_string(),
            name: "Demo Api".to_string(),
            url: "https://api.demo.local/data".to_string(),
            method: "GET".to_string(),
            params: "{\"page\":1}".to_string(),
            headers: "{\"Authorization\":\"Bearer demo\"}".to_string(),
        }];
        let databases = vec![CommonDatabaseRef {
            origin_id: "db-1".to_string(),
            ref_key: "common-database-ref-1".to_string(),
            ty: "MySQL".to_string(),
            ip: "127.0.0.1".to_string(),
            port: "3306".to_string(),
            database_name: "demo".to_string(),
            username: "root".to_string(),
            password: "secret".to_string(),
            url: String::new(),
            des: "runtime test".to_string(),
            allow_sql_script: true,
        }];

        let common_apis_plaintext =
            serde_json::to_vec(&common_apis).expect("serialize commonApis plaintext failed");
        let databases_plaintext =
            serde_json::to_vec(&databases).expect("serialize databases plaintext failed");

        let encrypted_common_apis =
            encrypt_with_passphrase_v1(&common_apis_plaintext, "export-passphrase")
                .expect("encrypt commonApis with passphrase failed");
        let encrypted_databases =
            encrypt_with_passphrase_v1(&databases_plaintext, "export-passphrase")
                .expect("encrypt databases with passphrase failed");

        let decrypted_common_apis =
            decrypt_with_passphrase_v1(&encrypted_common_apis, "export-passphrase")
                .expect("decrypt commonApis with passphrase failed");
        let decrypted_databases =
            decrypt_with_passphrase_v1(&encrypted_databases, "export-passphrase")
                .expect("decrypt databases with passphrase failed");

        let parsed_common_apis: Vec<CommonApiRef> = serde_json::from_slice(&decrypted_common_apis)
            .expect("parse decrypted commonApis failed");
        let parsed_databases: Vec<CommonDatabaseRef> =
            serde_json::from_slice(&decrypted_databases).expect("parse decrypted databases failed");

        assert!(encrypted_common_apis.starts_with(ENCRYPTED_FIELD_PREFIX_V1));
        assert!(encrypted_databases.starts_with(ENCRYPTED_FIELD_PREFIX_V1));
        assert_ne!(encrypted_common_apis, encrypted_databases);
        assert_eq!(
            serde_json::to_value(&parsed_common_apis).expect("serialize parsed commonApis failed"),
            serde_json::to_value(&common_apis).expect("serialize expected commonApis failed")
        );
        assert_eq!(
            serde_json::to_value(&parsed_databases).expect("serialize parsed databases failed"),
            serde_json::to_value(&databases).expect("serialize expected databases failed")
        );
    }
}
