/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 前后端桥接层的混合加密工具。
//!
//! 前端请求使用“RSA 加密 AES Key + AES-CBC 加密正文”的结构传入；
//! 后端响应也按同一结构用前端提供的公钥回包。该模块只负责协议加解密，
//! 不处理业务鉴权或密钥持久化。

use std::sync::OnceLock;

use aes::cipher::block_padding::UnpadError;
use aes::cipher::inout::PadError;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, InvalidLength, KeyIvInit};
use aes::Aes256;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use block_padding::Pkcs7;
use cbc::{Decryptor, Encryptor};
use rand::rngs::OsRng;
use rand::RngCore;
use rsa::pkcs8::{DecodePublicKey, EncodePublicKey};
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// 前后端加密传输使用的载荷格式。
pub struct EncryptResult {
    pub encrypted_data: String,
    pub encrypted_aes_key: String,
    pub iv: String,
}

/// 进程内 RSA 密钥对。
///
/// 私钥只保存在当前进程内存中，公钥提供给前端用于加密请求 AES Key。
struct ServerKeyPair {
    private: RsaPrivateKey,
    public_pem: String,
}

static SERVER_KEYS: OnceLock<ServerKeyPair> = OnceLock::new();

/// 懒生成服务端 RSA 密钥对。
///
/// 使用 `OnceLock` 保证同一进程内所有加密请求共享同一把临时公私钥。
fn server_keys() -> &'static ServerKeyPair {
    SERVER_KEYS.get_or_init(|| {
        let private = RsaPrivateKey::new(&mut OsRng, 2048).expect("generate rsa keypair");
        let public = RsaPublicKey::from(&private);
        let public_pem = public.to_public_key_pem(Default::default()).expect("public pem");
        ServerKeyPair { private, public_pem }
    })
}

/// 返回服务端公钥 PEM，供前端发起加密请求前获取。
pub fn server_public_key_pem() -> String {
    server_keys().public_pem.clone()
}

/// 解密前端发来的加密请求体。
///
/// 解密顺序是：先用服务端 RSA 私钥解出 AES Key，再用 AES-256-CBC 解正文。
pub fn decrypt_request(enc: EncryptResult) -> Result<String, String> {
    let encrypted_key_bytes = STANDARD
        .decode(enc.encrypted_aes_key)
        .map_err(|e| format!("crypto.decrypt_request: base64 decode encryptedAesKey failed: {e}"))?;
    let aes_key_b64_bytes = server_keys()
        .private
        .decrypt(Pkcs1v15Encrypt, &encrypted_key_bytes)
        .map_err(|e| format!("crypto.decrypt_request: rsa decrypt aesKey failed: {e}"))?;
    let aes_key_b64 = String::from_utf8(aes_key_b64_bytes).map_err(|e| format!("crypto.decrypt_request: aesKey utf8 decode failed: {e}"))?;

    // 前端传过来的 AES Key 自身也是 base64 字符串，RSA 解密后还需要再解一层。
    let aes_key = STANDARD
        .decode(aes_key_b64)
        .map_err(|e| format!("crypto.decrypt_request: base64 decode aesKey failed: {e}"))?;
    let iv = STANDARD
        .decode(enc.iv)
        .map_err(|e| format!("crypto.decrypt_request: base64 decode iv failed: {e}"))?;
    let cipher = STANDARD
        .decode(enc.encrypted_data)
        .map_err(|e| format!("crypto.decrypt_request: base64 decode encryptedData failed: {e}"))?;

    let mut buf = cipher;
    // CBC + PKCS7 要在可变缓冲区上原地解密并去 padding。
    let plaintext = Decryptor::<Aes256>::new_from_slices(&aes_key, &iv)
        .map_err(|e: InvalidLength| format!("crypto.decrypt_request: invalid key/iv length: {e}"))?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e: UnpadError| format!("crypto.decrypt_request: aes decrypt unpad failed: {e}"))?;

    String::from_utf8(plaintext.to_vec()).map_err(|e| format!("crypto.decrypt_request: plaintext utf8 decode failed: {e}"))
}

/// 使用前端公钥加密响应体。
///
/// 每次响应都会生成新的 AES Key 和 IV，降低重复明文产生相同密文的风险。
pub fn encrypt_response(client_public_key_b64: &str, plain_text: &str) -> Result<EncryptResult, String> {
    let client_public_pem = format!("-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n", chunk_b64(client_public_key_b64));
    let client_public =
        RsaPublicKey::from_public_key_pem(&client_public_pem).map_err(|e| format!("crypto.encrypt_response: parse client public key failed: {e}"))?;

    let mut aes_key = [0u8; 32];
    let mut iv = [0u8; 16];
    OsRng.fill_bytes(&mut aes_key);
    OsRng.fill_bytes(&mut iv);

    let plain_bytes = plain_text.as_bytes();
    let block_size = 16usize;
    let padded_len = ((plain_bytes.len() + block_size) / block_size) * block_size;
    let mut buf = vec![0u8; padded_len];
    buf[..plain_bytes.len()].copy_from_slice(plain_bytes);
    let cipher = Encryptor::<Aes256>::new_from_slices(&aes_key, &iv)
        .map_err(|e: InvalidLength| format!("crypto.encrypt_response: invalid key/iv length: {e}"))?
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain_bytes.len())
        .map_err(|e: PadError| format!("crypto.encrypt_response: aes encrypt pad failed: {e}"))?;

    // 与前端请求协议保持一致：RSA 加密的是 base64 后的 AES Key 字符串。
    let aes_key_b64 = STANDARD.encode(aes_key);
    let encrypted_aes_key = client_public
        .encrypt(&mut OsRng, Pkcs1v15Encrypt, aes_key_b64.as_bytes())
        .map_err(|e| format!("crypto.encrypt_response: rsa encrypt aesKey failed: {e}"))?;

    Ok(EncryptResult {
        encrypted_data: STANDARD.encode(cipher),
        encrypted_aes_key: STANDARD.encode(encrypted_aes_key),
        iv: STANDARD.encode(iv),
    })
}

/// 将裸 base64 公钥按 PEM 规范拆行。
fn chunk_b64(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + input.len() / 64 + 8);
    let mut i = 0;
    while i < input.len() {
        let end = (i + 64).min(input.len());
        out.push_str(&input[i..end]);
        out.push('\n');
        i = end;
    }
    out.trim_end().to_string()
}
