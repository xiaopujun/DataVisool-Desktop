/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 模型 API Key 本地密钥和解密逻辑。

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use aes::cipher::{BlockDecryptMut, InvalidLength, KeyIvInit};
use aes::Aes256;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use block_padding::Pkcs7;
use cbc::Decryptor;
use rand::rngs::OsRng;
use rand::RngCore;

use crate::app_state::AppState;
use crate::backend::ai_driven::model_invocation::types::ModelInvocationError;

/// API Key 落盘后的加密前缀。
const AI_MODEL_API_KEY_ENCRYPTION_PREFIX: &str = "enc:v1:";

/// 进程内缓存的 API Key 加密密钥。
static AI_MODEL_API_KEY_SECRET: OnceLock<Vec<u8>> = OnceLock::new();

/// 读取或创建本地 API Key 加密密钥。
fn load_or_create_ai_model_api_key_secret(app_state: &AppState) -> Result<&'static [u8], String> {
    if let Some(secret) = AI_MODEL_API_KEY_SECRET.get() {
        return Ok(secret.as_slice());
    }

    let path = app_state.ai_model_api_key_secret_file();
    let parent = path
        .parent()
        .ok_or_else(|| "aiModel.secretKey: invalid secret key path".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("aiModel.secretKey: create parent dir failed: {e}"))?;

    let secret = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => create_secret_file(&path)?,
        Err(err) => return Err(format!("aiModel.secretKey: read secret key failed: {err}")),
    };

    if secret.len() != 32 {
        return Err(format!("aiModel.secretKey: invalid secret key length {}, expected 32", secret.len()));
    }

    let _ = AI_MODEL_API_KEY_SECRET.set(secret);
    AI_MODEL_API_KEY_SECRET
        .get()
        .map(|value| value.as_slice())
        .ok_or_else(|| "aiModel.secretKey: initialize secret key cache failed".to_string())
}

/// 创建 API Key 加密密钥文件。
fn create_secret_file(path: &PathBuf) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; 32];
    OsRng.fill_bytes(&mut bytes);

    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(&bytes)
                .map_err(|e| format!("aiModel.secretKey: write secret key failed: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                    .map_err(|e| format!("aiModel.secretKey: set file permissions failed: {e}"))?;
            }
            Ok(bytes)
        }
        Err(open_err) if open_err.kind() == ErrorKind::AlreadyExists => {
            fs::read(path).map_err(|e| format!("aiModel.secretKey: read existing secret key failed: {e}"))
        }
        Err(open_err) => Err(format!("aiModel.secretKey: create secret key file failed: {open_err}")),
    }
}

/// 解密已落库的 API Key。
pub(super) fn decrypt_api_key_if_needed(app_state: &AppState, stored_value: Option<&str>) -> Result<Option<String>, ModelInvocationError> {
    let Some(stored_value) = stored_value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if !stored_value.starts_with(AI_MODEL_API_KEY_ENCRYPTION_PREFIX) {
        return Err(ModelInvocationError::validation("API Key 密文格式不合法，请重新配置"));
    }

    let payload = &stored_value[AI_MODEL_API_KEY_ENCRYPTION_PREFIX.len()..];
    let Some((iv_b64, cipher_b64)) = payload.split_once(':') else {
        return Err(ModelInvocationError::validation("API Key 密文格式不合法"));
    };
    let iv = STANDARD
        .decode(iv_b64)
        .map_err(|e| ModelInvocationError::validation(format!("API Key IV 解码失败：{e}")))?;
    let mut cipher = STANDARD
        .decode(cipher_b64)
        .map_err(|e| ModelInvocationError::validation(format!("API Key 密文解码失败：{e}")))?;
    let secret =
        load_or_create_ai_model_api_key_secret(app_state).map_err(|e| ModelInvocationError::validation(format!("读取 API Key 密钥失败：{e}")))?;

    let plain = Decryptor::<Aes256>::new_from_slices(secret, &iv)
        .map_err(|e: InvalidLength| ModelInvocationError::validation(format!("API Key 解密参数不合法：{e}")))?
        .decrypt_padded_mut::<Pkcs7>(&mut cipher)
        .map_err(|e| ModelInvocationError::validation(format!("API Key 解密失败：{e}")))?;
    let plain = String::from_utf8(plain.to_vec()).map_err(|e| ModelInvocationError::validation(format!("API Key UTF-8 解码失败：{e}")))?;
    Ok(Some(plain))
}
