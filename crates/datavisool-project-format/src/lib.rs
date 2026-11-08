/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportManifest {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub format_version: u32,
    #[serde(default)]
    pub generator: String,
    #[serde(default)]
    pub generated_at_ms: u64,
    pub project: ManifestProject,
    #[serde(default)]
    pub projects: Vec<ManifestProjectNode>,
    pub assets: ManifestAssets,
    #[serde(default)]
    pub common_datasource: ManifestCommonDataSources,
    pub files: Vec<ManifestFile>,
    pub warnings_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestProject {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub data_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestProjectNode {
    #[serde(default)]
    pub source_project_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub source_pid: Option<String>,
    #[serde(default)]
    pub data_path: String,
    #[serde(default)]
    pub is_entry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestAssets {
    #[serde(default)]
    pub images: Vec<AssetBlobRef>,
    #[serde(default)]
    pub fonts: Vec<AssetBlobRef>,
    #[serde(default)]
    pub videos: Vec<AssetBlobRef>,
    #[serde(default)]
    pub components: Vec<AssetBlobRef>,
    #[serde(default)]
    pub dynamic_scripts: Vec<DynamicScriptRef>,
    #[serde(default)]
    pub models: Vec<ModelPackageRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestCommonDataSources {
    #[serde(default)]
    pub common_apis: EncryptedOrPlain<Vec<CommonApiRef>>,
    #[serde(default)]
    pub databases: EncryptedOrPlain<Vec<CommonDatabaseRef>>,
}

pub const ENCRYPTED_FIELD_PREFIX_V1: &str = "enc:v1:";
pub const ENCRYPTED_ENVELOPE_ALG_A256_GCM: &str = "A256GCM";
pub const ENCRYPTED_ENVELOPE_KDF_ARGON2_ID: &str = "Argon2id";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum EncryptedOrPlain<T> {
    Plain(T),
    // TODO(legacy-compat): 停止支持历史字段级加密项目包后，移除此分支及导入解密逻辑。
    Encrypted(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedEnvelopeV1 {
    #[serde(default)]
    pub alg: String,
    #[serde(default)]
    pub kdf: String,
    #[serde(default)]
    pub salt: String,
    #[serde(default)]
    pub nonce: String,
    #[serde(default)]
    pub ciphertext: String,
}

impl<T: Default> Default for EncryptedOrPlain<T> {
    fn default() -> Self {
        Self::Plain(T::default())
    }
}

impl<T> EncryptedOrPlain<T> {
    pub fn as_plain(&self) -> Option<&T> {
        match self {
            Self::Plain(value) => Some(value),
            Self::Encrypted(_) => None,
        }
    }

    pub fn as_encrypted(&self) -> Option<&str> {
        match self {
            Self::Plain(_) => None,
            Self::Encrypted(value) => Some(value),
        }
    }
}

impl<T> From<T> for EncryptedOrPlain<T> {
    fn from(value: T) -> Self {
        Self::Plain(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommonApiRef {
    #[serde(default)]
    pub origin_id: String,
    #[serde(default)]
    pub ref_key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub params: String,
    #[serde(default)]
    pub headers: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommonDatabaseRef {
    #[serde(default)]
    pub origin_id: String,
    #[serde(default)]
    pub ref_key: String,
    #[serde(default, rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub database_name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub des: String,
    #[serde(default)]
    pub allow_sql_script: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetBlobRef {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub original: String,
    #[serde(default)]
    pub blob: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DynamicScriptRef {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub ty: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub owner_source_project_id: Option<String>,
    pub original: Option<String>,
    pub blob: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelPackageRef {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub original: String,
    #[serde(default)]
    pub package_id: String,
    #[serde(default)]
    pub entry: String,
    #[serde(default)]
    pub files: Vec<ModelFileRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelFileRef {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub blob: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("invalid format")]
    InvalidFormat,
    #[error("missing formatVersion")]
    MissingFormatVersion,
    #[error("formatVersion too old: {0}")]
    FormatVersionTooOld(u32),
    #[error("formatVersion too new: {0}")]
    FormatVersionTooNew(u32),
    #[error("missing project.id")]
    MissingProjectId,
    #[error("missing project.name")]
    MissingProjectName,
    #[error("missing projects for formatVersion 3")]
    MissingProjects,
    #[error("missing projects[{0}].sourceProjectId")]
    MissingProjectNodeSourceProjectId(usize),
    #[error("missing projects[{0}].name")]
    MissingProjectNodeName(usize),
    #[error("missing projects[{0}].dataPath")]
    MissingProjectNodeDataPath(usize),
    #[error("missing entry project node")]
    MissingEntryProjectNode,
    #[error("multiple entry project nodes: {0}")]
    MultipleEntryProjectNodes(usize),
    #[error("entry project node does not match manifest.project.id")]
    EntryProjectIdMismatch,
}

pub const PROJECT_FORMAT: &str = "datavisool-project";

pub fn is_supported_project_format(format: &str) -> bool {
    // TODO(legacy-compat): 停止支持 LCP 项目包后，移除旧格式标记的识别。
    format == PROJECT_FORMAT || datavisool_compat::is_legacy_project_format(format)
}

pub const SUPPORTED_MIN_FORMAT_VERSION: u32 = 2;
pub const SUPPORTED_MAX_FORMAT_VERSION: u32 = 3;

pub fn validate_manifest(manifest: &ExportManifest) -> Result<(), ManifestError> {
    if !is_supported_project_format(&manifest.format) {
        return Err(ManifestError::InvalidFormat);
    }
    let v = manifest.format_version;
    if v == 0 {
        return Err(ManifestError::MissingFormatVersion);
    }
    if v < SUPPORTED_MIN_FORMAT_VERSION {
        return Err(ManifestError::FormatVersionTooOld(v));
    }
    if v > SUPPORTED_MAX_FORMAT_VERSION {
        return Err(ManifestError::FormatVersionTooNew(v));
    }
    if manifest.project.id.trim().is_empty() {
        return Err(ManifestError::MissingProjectId);
    }
    if manifest.project.name.trim().is_empty() {
        return Err(ManifestError::MissingProjectName);
    }
    if v >= 3 {
        if manifest.projects.is_empty() {
            return Err(ManifestError::MissingProjects);
        }
        let mut entry_indexes: Vec<usize> = Vec::new();
        for (index, node) in manifest.projects.iter().enumerate() {
            if node.source_project_id.trim().is_empty() {
                return Err(ManifestError::MissingProjectNodeSourceProjectId(index));
            }
            if node.name.trim().is_empty() {
                return Err(ManifestError::MissingProjectNodeName(index));
            }
            if node.data_path.trim().is_empty() {
                return Err(ManifestError::MissingProjectNodeDataPath(index));
            }
            if node.is_entry {
                entry_indexes.push(index);
            }
        }
        if entry_indexes.is_empty() {
            return Err(ManifestError::MissingEntryProjectNode);
        }
        if entry_indexes.len() > 1 {
            return Err(ManifestError::MultipleEntryProjectNodes(
                entry_indexes.len(),
            ));
        }
        let entry_node = &manifest.projects[entry_indexes[0]];
        if entry_node.source_project_id.trim() != manifest.project.id.trim() {
            return Err(ManifestError::EntryProjectIdMismatch);
        }
    }
    Ok(())
}

pub fn resolve_project_data_path(manifest: &ExportManifest) -> String {
    let p = manifest.project.data_path.trim();
    if p.is_empty() {
        "project/project.json".to_string()
    } else {
        p.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_manifest_value() -> serde_json::Value {
        json!({
            "format": "light-chaser-project",
            "formatVersion": 2,
            "generator": "test",
            "generatedAtMs": 0,
            "project": {
                "id": "p1",
                "name": "demo",
                "dataPath": "project/project.json"
            },
            "projects": [],
            "assets": {
                "images": [],
                "fonts": [],
                "videos": [],
                "components": [],
                "dynamicScripts": [],
                "models": []
            },
            "files": []
        })
    }

    #[test]
    fn deserialize_manifest_without_common_datasource_uses_default() {
        let manifest: ExportManifest = serde_json::from_value(base_manifest_value())
            .expect("manifest without commonDatasource should parse");

        assert!(manifest
            .common_datasource
            .common_apis
            .as_plain()
            .expect("commonApis should default to plain array")
            .is_empty());
        assert!(manifest
            .common_datasource
            .databases
            .as_plain()
            .expect("databases should default to plain array")
            .is_empty());
        assert_eq!(resolve_project_data_path(&manifest), "project/project.json");
        assert!(manifest.projects.is_empty());
    }

    #[test]
    fn deserialize_manifest_with_common_datasource() {
        let mut value = base_manifest_value();
        value["commonDatasource"] = json!({
            "commonApis": [{
                "originId": "1",
                "refKey": "api-ref-1",
                "name": "api",
                "url": "https://example.com",
                "method": "GET",
                "params": "{}",
                "headers": "{}"
            }],
            "databases": [{
                "originId": "2",
                "refKey": "db-ref-2",
                "type": "MySQL",
                "ip": "127.0.0.1",
                "port": "3306",
                "databaseName": "demo",
                "username": "root",
                "password": "pwd",
                "url": "",
                "des": "db"
            }]
        });

        let manifest: ExportManifest =
            serde_json::from_value(value).expect("manifest with commonDatasource should parse");

        let common_apis = manifest
            .common_datasource
            .common_apis
            .as_plain()
            .expect("commonApis should parse as plain array");
        assert_eq!(common_apis.len(), 1);
        assert_eq!(common_apis[0].origin_id, "1");
        assert_eq!(common_apis[0].ref_key, "api-ref-1");
        let databases = manifest
            .common_datasource
            .databases
            .as_plain()
            .expect("databases should parse as plain array");
        assert_eq!(databases.len(), 1);
        assert_eq!(databases[0].database_name, "demo");
        assert_eq!(databases[0].ref_key, "db-ref-2");
        assert_eq!(databases[0].ty, "MySQL");
    }

    #[test]
    fn deserialize_manifest_with_encrypted_common_apis() {
        let mut value = base_manifest_value();
        value["commonDatasource"] = json!({
            "commonApis": "enc:v1:encrypted-common-api-payload",
            "databases": []
        });

        let manifest: ExportManifest =
            serde_json::from_value(value).expect("manifest with encrypted commonApis should parse");

        assert_eq!(
            manifest.common_datasource.common_apis.as_encrypted(),
            Some("enc:v1:encrypted-common-api-payload")
        );
    }

    #[test]
    fn deserialize_manifest_with_encrypted_databases() {
        let mut value = base_manifest_value();
        value["commonDatasource"] = json!({
            "commonApis": [],
            "databases": "enc:v1:encrypted-database-payload"
        });

        let manifest: ExportManifest =
            serde_json::from_value(value).expect("manifest with encrypted databases should parse");

        assert_eq!(
            manifest.common_datasource.databases.as_encrypted(),
            Some("enc:v1:encrypted-database-payload")
        );
    }

    #[test]
    fn deserialize_legacy_and_encrypted_manifest_formats_are_both_supported() {
        let legacy_manifest: ExportManifest =
            serde_json::from_value(base_manifest_value()).expect("legacy manifest should parse");
        assert!(legacy_manifest
            .common_datasource
            .common_apis
            .as_plain()
            .expect("legacy commonApis should default to plain array")
            .is_empty());
        assert!(legacy_manifest
            .common_datasource
            .databases
            .as_plain()
            .expect("legacy databases should default to plain array")
            .is_empty());

        let mut encrypted_value = base_manifest_value();
        encrypted_value["commonDatasource"] = json!({
            "commonApis": "enc:v1:encrypted-common-api-payload",
            "databases": "enc:v1:encrypted-database-payload"
        });

        let encrypted_manifest: ExportManifest =
            serde_json::from_value(encrypted_value).expect("encrypted manifest should parse");

        assert_eq!(encrypted_manifest.project.id, "p1");
        assert_eq!(
            encrypted_manifest
                .common_datasource
                .common_apis
                .as_encrypted(),
            Some("enc:v1:encrypted-common-api-payload")
        );
        assert_eq!(
            encrypted_manifest
                .common_datasource
                .databases
                .as_encrypted(),
            Some("enc:v1:encrypted-database-payload")
        );
    }

    #[test]
    fn serialize_manifest_with_common_datasource_uses_expected_json_keys() {
        let manifest = ExportManifest {
            format: "light-chaser-project".to_string(),
            format_version: 2,
            generator: "test".to_string(),
            generated_at_ms: 0,
            project: ManifestProject {
                id: "p1".to_string(),
                name: "demo".to_string(),
                data_path: "project/project.json".to_string(),
            },
            projects: vec![],
            assets: ManifestAssets::default(),
            common_datasource: ManifestCommonDataSources {
                common_apis: vec![CommonApiRef {
                    origin_id: "1".to_string(),
                    ref_key: "api-ref-1".to_string(),
                    name: "api".to_string(),
                    url: "https://example.com".to_string(),
                    method: "GET".to_string(),
                    params: "{}".to_string(),
                    headers: "{}".to_string(),
                }]
                .into(),
                databases: vec![CommonDatabaseRef {
                    origin_id: "2".to_string(),
                    ref_key: "db-ref-2".to_string(),
                    ty: "MySQL".to_string(),
                    ip: "127.0.0.1".to_string(),
                    port: "3306".to_string(),
                    database_name: "demo".to_string(),
                    username: "root".to_string(),
                    password: "pwd".to_string(),
                    url: "".to_string(),
                    des: "db".to_string(),
                    allow_sql_script: true,
                }]
                .into(),
            },
            files: vec![],
            warnings_path: None,
        };

        let value = serde_json::to_value(&manifest).expect("serialize manifest failed");

        assert_eq!(value["commonDatasource"]["commonApis"][0]["originId"], "1");
        assert_eq!(
            value["commonDatasource"]["commonApis"][0]["refKey"],
            "api-ref-1"
        );
        assert_eq!(value["commonDatasource"]["databases"][0]["originId"], "2");
        assert_eq!(
            value["commonDatasource"]["databases"][0]["refKey"],
            "db-ref-2"
        );
        assert_eq!(value["commonDatasource"]["databases"][0]["type"], "MySQL");
        assert_eq!(
            value["commonDatasource"]["databases"][0]["databaseName"],
            "demo"
        );
        assert_eq!(
            value["commonDatasource"]["databases"][0]["allowSqlScript"],
            true
        );
    }

    #[test]
    fn validate_manifest_accepts_v3_project_graph() {
        let mut value = base_manifest_value();
        value["formatVersion"] = json!(3);
        value["project"] = json!({
            "id": "A1",
            "name": "Child A1",
            "dataPath": "projects/A1/project.json"
        });
        value["projects"] = json!([
            {
                "sourceProjectId": "A",
                "name": "Root A",
                "sourcePid": null,
                "dataPath": "projects/A/project.json",
                "isEntry": false
            },
            {
                "sourceProjectId": "A1",
                "name": "Child A1",
                "sourcePid": "A",
                "dataPath": "projects/A1/project.json",
                "isEntry": true
            }
        ]);
        value["assets"]["dynamicScripts"] = json!([
            {
                "name": "Project Script",
                "type": "LOCAL",
                "scope": "PROJECT",
                "ownerSourceProjectId": "A1",
                "original": "foo.js",
                "blob": "blobs/foo.js",
                "url": null
            }
        ]);

        let manifest: ExportManifest =
            serde_json::from_value(value).expect("v3 manifest should deserialize");

        validate_manifest(&manifest).expect("v3 manifest should validate");
        assert_eq!(manifest.projects.len(), 2);
        assert_eq!(manifest.projects[1].source_pid.as_deref(), Some("A"));
        assert_eq!(manifest.assets.dynamic_scripts[0].scope, "PROJECT");
        assert_eq!(
            manifest.assets.dynamic_scripts[0]
                .owner_source_project_id
                .as_deref(),
            Some("A1")
        );
    }

    #[test]
    fn validate_manifest_rejects_v3_without_projects() {
        let mut value = base_manifest_value();
        value["formatVersion"] = json!(3);

        let manifest: ExportManifest =
            serde_json::from_value(value).expect("v3 manifest without projects should deserialize");

        assert!(matches!(
            validate_manifest(&manifest),
            Err(ManifestError::MissingProjects)
        ));
    }

    #[test]
    fn validate_manifest_rejects_v3_project_node_missing_required_fields() {
        let mut value = base_manifest_value();
        value["formatVersion"] = json!(3);
        value["projects"] = json!([
            {
                "sourceProjectId": "",
                "name": "Broken Node",
                "sourcePid": null,
                "dataPath": "",
                "isEntry": true
            }
        ]);

        let manifest: ExportManifest =
            serde_json::from_value(value).expect("invalid v3 manifest should deserialize");

        assert!(matches!(
            validate_manifest(&manifest),
            Err(ManifestError::MissingProjectNodeSourceProjectId(0))
        ));
    }

    #[test]
    fn validate_manifest_rejects_v3_without_entry_node() {
        let mut value = base_manifest_value();
        value["formatVersion"] = json!(3);
        value["projects"] = json!([
            {
                "sourceProjectId": "A",
                "name": "Root A",
                "sourcePid": null,
                "dataPath": "projects/A/project.json",
                "isEntry": false
            }
        ]);

        let manifest: ExportManifest = serde_json::from_value(value)
            .expect("v3 manifest without entry node should deserialize");

        assert!(matches!(
            validate_manifest(&manifest),
            Err(ManifestError::MissingEntryProjectNode)
        ));
    }

    #[test]
    fn validate_manifest_rejects_v3_when_entry_node_mismatches_project_id() {
        let mut value = base_manifest_value();
        value["formatVersion"] = json!(3);
        value["project"] = json!({
            "id": "ENTRY",
            "name": "Entry",
            "dataPath": "projects/ENTRY/project.json"
        });
        value["projects"] = json!([
            {
                "sourceProjectId": "OTHER",
                "name": "Other",
                "sourcePid": null,
                "dataPath": "projects/OTHER/project.json",
                "isEntry": true
            }
        ]);

        let manifest: ExportManifest =
            serde_json::from_value(value).expect("mismatched v3 manifest should deserialize");

        assert!(matches!(
            validate_manifest(&manifest),
            Err(ManifestError::EntryProjectIdMismatch)
        ));
    }

    #[test]
    fn serialize_encrypted_envelope_v1_uses_expected_json_keys() {
        let envelope = EncryptedEnvelopeV1 {
            alg: ENCRYPTED_ENVELOPE_ALG_A256_GCM.to_string(),
            kdf: ENCRYPTED_ENVELOPE_KDF_ARGON2_ID.to_string(),
            salt: "salt-base64".to_string(),
            nonce: "nonce-base64".to_string(),
            ciphertext: "ciphertext-base64".to_string(),
        };

        let value = serde_json::to_value(&envelope).expect("serialize encrypted envelope failed");

        assert_eq!(value["alg"], ENCRYPTED_ENVELOPE_ALG_A256_GCM);
        assert_eq!(value["kdf"], ENCRYPTED_ENVELOPE_KDF_ARGON2_ID);
        assert_eq!(value["salt"], "salt-base64");
        assert_eq!(value["nonce"], "nonce-base64");
        assert_eq!(value["ciphertext"], "ciphertext-base64");
    }

    #[test]
    fn deserialize_encrypted_envelope_v1() {
        let value = json!({
            "alg": "A256GCM",
            "kdf": "Argon2id",
            "salt": "salt-base64",
            "nonce": "nonce-base64",
            "ciphertext": "ciphertext-base64"
        });

        let envelope: EncryptedEnvelopeV1 =
            serde_json::from_value(value).expect("deserialize encrypted envelope failed");

        assert_eq!(envelope.alg, ENCRYPTED_ENVELOPE_ALG_A256_GCM);
        assert_eq!(envelope.kdf, ENCRYPTED_ENVELOPE_KDF_ARGON2_ID);
        assert_eq!(envelope.salt, "salt-base64");
        assert_eq!(envelope.nonce, "nonce-base64");
        assert_eq!(envelope.ciphertext, "ciphertext-base64");
        assert_eq!(ENCRYPTED_FIELD_PREFIX_V1, "enc:v1:");
    }
}
