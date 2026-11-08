/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 安装包组件说明资源校验与受控内容搜索。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{path::BaseDirectory, Manager};
use tokio::sync::watch;

use super::{component_document_location, has_component_document, COMPONENT_DOCS_CORPUS_VERSION, EMBEDDED_COMPONENT_DOCS};
use crate::backend::ai_driven::ripgrep::{
    RipgrepError, RipgrepSearchOptions, RipgrepSearchService, SearchCaseMode, SearchCorpus, SearchMode, SearchSubmatch, SearchTruncationReason,
};

const COMPONENT_DOCS_RESOURCE_PATH: &str = "resources/ai/component-optimization";
const MAX_COMPONENT_TYPES: usize = 20;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

#[derive(Clone)]
pub(crate) struct ComponentDocumentSearchService {
    app_handle: Option<tauri::AppHandle>,
    ripgrep: RipgrepSearchService,
    verified: Arc<Mutex<Option<VerifiedResource>>>,
    #[cfg(test)]
    resource_root_override: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct ComponentDocumentSearchRequest {
    pub query: String,
    pub mode: SearchMode,
    pub case_mode: SearchCaseMode,
    pub component_types: Vec<String>,
    pub max_results: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComponentDocumentSearchMatch {
    pub component_type: String,
    pub file_name: String,
    pub section: &'static str,
    pub line: u64,
    pub section_line: usize,
    pub column: Option<u64>,
    pub text: String,
    pub submatches: Vec<SearchSubmatch>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComponentDocumentSearchResponse {
    pub corpus_version: &'static str,
    pub matches: Vec<ComponentDocumentSearchMatch>,
    pub truncated: bool,
    pub truncation_reason: Option<SearchTruncationReason>,
    pub elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ComponentDocumentSearchErrorKind {
    InvalidRequest,
    ResourceMissing,
    ResourceMismatch,
    InvalidResult,
    Ripgrep,
}

#[derive(Debug)]
pub(crate) struct ComponentDocumentSearchError {
    pub(crate) kind: ComponentDocumentSearchErrorKind,
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) ripgrep: Option<RipgrepError>,
}

impl ComponentDocumentSearchError {
    fn new(kind: ComponentDocumentSearchErrorKind, code: &'static str, message: impl Into<String>) -> Self {
        Self { kind, code, message: message.into(), ripgrep: None }
    }

    fn missing(message: impl Into<String>) -> Self {
        Self::new(ComponentDocumentSearchErrorKind::ResourceMissing, "agent.component_document.search_resource_missing", message)
    }

    fn mismatch(message: impl Into<String>) -> Self {
        Self::new(
            ComponentDocumentSearchErrorKind::ResourceMismatch,
            "agent.component_document.search_resource_mismatch",
            message,
        )
    }

    fn invalid_result(message: impl Into<String>) -> Self {
        Self::new(ComponentDocumentSearchErrorKind::InvalidResult, "agent.component_document.search_result_invalid", message)
    }
}

impl std::fmt::Display for ComponentDocumentSearchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ComponentDocumentSearchError {}

impl From<RipgrepError> for ComponentDocumentSearchError {
    fn from(error: RipgrepError) -> Self {
        Self {
            kind: ComponentDocumentSearchErrorKind::Ripgrep,
            code: error.code,
            message: error.message.clone(),
            ripgrep: Some(error),
        }
    }
}

impl ComponentDocumentSearchService {
    pub(crate) fn new(app_handle: tauri::AppHandle, ripgrep: RipgrepSearchService) -> Self {
        Self {
            app_handle: Some(app_handle),
            ripgrep,
            verified: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            resource_root_override: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(ripgrep: RipgrepSearchService, resource_root: PathBuf) -> Self {
        Self {
            app_handle: None,
            ripgrep,
            verified: Arc::new(Mutex::new(None)),
            resource_root_override: Some(resource_root),
        }
    }

    pub(crate) async fn search(
        &self,
        request: ComponentDocumentSearchRequest,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<ComponentDocumentSearchResponse, ComponentDocumentSearchError> {
        validate_request(&request)?;
        let root = self.resource_root()?;
        verify_resource_directory(&root, &self.verified)?;
        let globs = if request.component_types.is_empty() {
            vec!["*.md".to_string()]
        } else {
            request
                .component_types
                .iter()
                .map(|component_type| format!("{component_type}.md"))
                .collect()
        };
        let output = self
            .ripgrep
            .search_component_documents(
                SearchCorpus::component_documents(root),
                RipgrepSearchOptions {
                    query: request.query,
                    mode: request.mode,
                    case_mode: request.case_mode,
                    globs,
                    include_hidden: false,
                    respect_ignore_files: true,
                    max_results: request.max_results,
                },
                cancel_rx,
            )
            .await?;
        let mut matches = Vec::with_capacity(output.matches.len());
        for matched in output.matches {
            if matched.path.contains('/') || !matched.path.ends_with(".md") {
                return Err(ComponentDocumentSearchError::invalid_result("组件说明命中路径无效"));
            }
            let component_type = matched.path.trim_end_matches(".md");
            if !has_component_document(component_type) {
                return Err(ComponentDocumentSearchError::invalid_result("组件说明命中未登记的组件类型"));
            }
            let line = matched
                .line
                .ok_or_else(|| ComponentDocumentSearchError::invalid_result("组件说明命中缺少行号"))?;
            let (section, section_line) = component_document_location(component_type, line)
                .ok_or_else(|| ComponentDocumentSearchError::invalid_result("组件说明命中无法映射章节"))?;
            matches.push(ComponentDocumentSearchMatch {
                component_type: component_type.to_string(),
                file_name: matched.path,
                section: section.key(),
                line,
                section_line,
                column: matched.column,
                text: matched.text,
                submatches: matched.submatches,
            });
        }
        let mut response = ComponentDocumentSearchResponse {
            corpus_version: COMPONENT_DOCS_CORPUS_VERSION,
            matches,
            truncated: output.truncation_reason.is_some(),
            truncation_reason: output.truncation_reason,
            elapsed_ms: output.elapsed_ms,
        };
        fit_response(&mut response)?;
        Ok(response)
    }

    fn resource_root(&self) -> Result<PathBuf, ComponentDocumentSearchError> {
        #[cfg(test)]
        if let Some(root) = &self.resource_root_override {
            return Ok(root.clone());
        }
        self.app_handle
            .as_ref()
            .ok_or_else(|| ComponentDocumentSearchError::missing("组件说明资源根目录不可用"))?
            .path()
            .resolve(COMPONENT_DOCS_RESOURCE_PATH, BaseDirectory::Resource)
            .map_err(|_| ComponentDocumentSearchError::missing("安装包缺少组件说明资源目录"))
    }
}

fn validate_request(request: &ComponentDocumentSearchRequest) -> Result<(), ComponentDocumentSearchError> {
    if request.component_types.len() > MAX_COMPONENT_TYPES {
        return Err(ComponentDocumentSearchError::new(
            ComponentDocumentSearchErrorKind::InvalidRequest,
            "agent.component_document.search_invalid_request",
            format!("componentTypes 最多 {MAX_COMPONENT_TYPES} 项"),
        ));
    }
    let mut seen = BTreeSet::new();
    for component_type in &request.component_types {
        if !has_component_document(component_type) {
            return Err(ComponentDocumentSearchError::new(
                ComponentDocumentSearchErrorKind::InvalidRequest,
                "agent.component_document.search_invalid_request",
                format!("未登记组件资料：{component_type}"),
            ));
        }
        if !seen.insert(component_type) {
            return Err(ComponentDocumentSearchError::new(
                ComponentDocumentSearchErrorKind::InvalidRequest,
                "agent.component_document.search_invalid_request",
                format!("componentTypes 包含重复项：{component_type}"),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VerifiedResource {
    root: PathBuf,
    fingerprint: Vec<ResourceFingerprint>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ResourceFingerprint {
    file_name: String,
    byte_size: u64,
    modified_nanos: u128,
}

fn verify_resource_directory(root: &Path, cache: &Mutex<Option<VerifiedResource>>) -> Result<(), ComponentDocumentSearchError> {
    let root_metadata = fs::symlink_metadata(root).map_err(|_| ComponentDocumentSearchError::missing("安装包缺少组件说明资源目录"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.file_type().is_dir() {
        return Err(ComponentDocumentSearchError::mismatch("组件说明资源根目录类型无效"));
    }
    let expected = EMBEDDED_COMPONENT_DOCS
        .iter()
        .map(|(component_type, _, byte_size, sha256)| (format!("{component_type}.md"), (*byte_size, *sha256)))
        .collect::<BTreeMap<_, _>>();
    let mut fingerprints = Vec::new();
    let entries = fs::read_dir(root).map_err(|_| ComponentDocumentSearchError::missing("无法读取组件说明资源目录"))?;
    for entry in entries {
        let entry = entry.map_err(|_| ComponentDocumentSearchError::mismatch("无法读取组件说明资源项"))?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| ComponentDocumentSearchError::mismatch("无法检查组件说明资源项"))?;
        let file_name = entry
            .file_name()
            .into_string()
            .map_err(|_| ComponentDocumentSearchError::mismatch("组件说明资源文件名不是 UTF-8"))?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() || !expected.contains_key(&file_name) {
            return Err(ComponentDocumentSearchError::mismatch(format!("组件说明资源包含未登记项：{file_name}")));
        }
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        fingerprints.push(ResourceFingerprint { file_name, byte_size: metadata.len(), modified_nanos });
    }
    fingerprints.sort();
    if fingerprints.len() != expected.len() || fingerprints.iter().any(|entry| !expected.contains_key(&entry.file_name)) {
        return Err(ComponentDocumentSearchError::mismatch("组件说明资源文件集合与嵌入清单不一致"));
    }
    {
        let verified = cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if verified
            .as_ref()
            .is_some_and(|verified| verified.root == root && verified.fingerprint == fingerprints)
        {
            return Ok(());
        }
    }
    for fingerprint in &fingerprints {
        let (expected_size, expected_sha256) = expected[&fingerprint.file_name];
        if fingerprint.byte_size != expected_size {
            return Err(ComponentDocumentSearchError::mismatch(format!("组件说明资源大小不一致：{}", fingerprint.file_name)));
        }
        let bytes = fs::read(root.join(&fingerprint.file_name))
            .map_err(|_| ComponentDocumentSearchError::mismatch(format!("无法读取组件说明资源：{}", fingerprint.file_name)))?;
        let actual = format!("{:x}", Sha256::digest(bytes));
        if actual != expected_sha256 {
            return Err(ComponentDocumentSearchError::mismatch(format!("组件说明资源哈希不一致：{}", fingerprint.file_name)));
        }
    }
    *cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(VerifiedResource { root: root.to_path_buf(), fingerprint: fingerprints });
    Ok(())
}

fn fit_response(response: &mut ComponentDocumentSearchResponse) -> Result<(), ComponentDocumentSearchError> {
    while serde_json::to_vec(response)
        .map_err(|_| ComponentDocumentSearchError::invalid_result("组件说明搜索结果无法序列化"))?
        .len()
        > MAX_RESPONSE_BYTES
    {
        if response.matches.pop().is_none() {
            return Err(ComponentDocumentSearchError::invalid_result("组件说明搜索响应超过硬限制"));
        }
        response.truncated = true;
        response.truncation_reason = Some(SearchTruncationReason::ResponseLimit);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai_driven::ripgrep::{bundled_test_binary, RipgrepLimits};

    fn copied_resource_root(name: &str) -> PathBuf {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/ai/component-optimization");
        let root = std::env::temp_dir().join(format!("datavisool-component-doc-search-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        root
    }

    #[test]
    fn source_component_resources_match_embedded_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/ai/component-optimization");
        verify_resource_directory(&root, &Mutex::new(None)).unwrap();
    }

    #[test]
    fn resource_verification_rejects_extra_files() {
        let root = std::env::temp_dir().join(format!("datavisool-component-doc-search-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("extra.md"), "extra").unwrap();
        let error = verify_resource_directory(&root, &Mutex::new(None)).unwrap_err();
        assert_eq!(error.code, "agent.component_document.search_resource_mismatch");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resource_verification_rejects_missing_and_tampered_files() {
        let missing_root = copied_resource_root("missing");
        fs::remove_file(missing_root.join("BaseText.md")).unwrap();
        assert_eq!(
            verify_resource_directory(&missing_root, &Mutex::new(None)).unwrap_err().code,
            "agent.component_document.search_resource_mismatch"
        );
        let _ = fs::remove_dir_all(missing_root);

        let tampered_root = copied_resource_root("tampered");
        let cache = Mutex::new(None);
        verify_resource_directory(&tampered_root, &cache).unwrap();
        fs::write(tampered_root.join("BaseText.md"), "tampered").unwrap();
        assert_eq!(
            verify_resource_directory(&tampered_root, &cache).unwrap_err().code,
            "agent.component_document.search_resource_mismatch"
        );
        let _ = fs::remove_dir_all(tampered_root);
    }

    #[cfg(unix)]
    #[test]
    fn resource_verification_rejects_symlinked_files() {
        use std::os::unix::fs::symlink;

        let root = copied_resource_root("symlink");
        fs::remove_file(root.join("BaseText.md")).unwrap();
        symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/ai/component-optimization/BaseText.md"),
            root.join("BaseText.md"),
        )
        .unwrap();
        assert_eq!(
            verify_resource_directory(&root, &Mutex::new(None)).unwrap_err().code,
            "agent.component_document.search_resource_mismatch"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn searches_registered_documents_and_maps_authoritative_section() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/ai/component-optimization");
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let service = ComponentDocumentSearchService::for_test(
            RipgrepSearchService::for_test(app.handle().clone(), bundled_test_binary(), RipgrepLimits::default()),
            root,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let response = service
            .search(
                ComponentDocumentSearchRequest {
                    query: "data(Patch)".to_string(),
                    mode: SearchMode::Regex,
                    case_mode: SearchCaseMode::Sensitive,
                    component_types: vec!["BaseText".to_string()],
                    max_results: 10,
                },
                cancel_rx,
            )
            .await
            .unwrap();

        assert_eq!(response.corpus_version, COMPONENT_DOCS_CORPUS_VERSION);
        assert!(!response.matches.is_empty());
        assert!(response
            .matches
            .iter()
            .all(|matched| matched.component_type == "BaseText" && matched.section == "dataFormatRequirements"));

        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let all_documents = service
            .search(
                ComponentDocumentSearchRequest {
                    query: "## 组件信息".to_string(),
                    mode: SearchMode::Literal,
                    case_mode: SearchCaseMode::Sensitive,
                    component_types: Vec::new(),
                    max_results: 100,
                },
                cancel_rx,
            )
            .await
            .unwrap();
        assert_eq!(
            all_documents
                .matches
                .iter()
                .map(|matched| matched.component_type.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            48
        );
    }
}
