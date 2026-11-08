/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! AI 任务校验诊断和修复记录存储。
//!
//! 每条记录使用独立 JSON 文件保存，避免追加写在任务取消或进程异常时损坏全部
//! 历史。文件名由后端生成，记录中的模型可见 ID 不参与物理路径计算。

use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use super::layout::WorkspaceLayout;
use super::manager::{write_bytes_atomically, WorkspaceHandle};
use super::types::{DiagnosticRecord, WorkspaceError, WorkspaceResult};
use crate::backend::ai_driven::time::now_ms;

/// 诊断记录文件序号，确保同一毫秒内的记录不会重名。
static DIAGNOSTIC_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 工作区诊断记录仓库。
#[derive(Clone, Debug)]
pub(crate) struct DiagnosticStore {
    layout: WorkspaceLayout,
}

impl DiagnosticStore {
    /// 从已创建或打开的工作区构造诊断仓库。
    pub(crate) fn new(workspace: &WorkspaceHandle) -> Self {
        Self { layout: workspace.layout.clone() }
    }

    /// 保存一条诊断或修复记录。
    pub(crate) fn append(&self, record: &DiagnosticRecord) -> WorkspaceResult<()> {
        let timestamp = now_ms();
        let sequence = DIAGNOSTIC_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = self
            .layout
            .diagnostics_dir()
            .join(format!("{timestamp:020}-{}-{sequence:020}.json", std::process::id()));
        let bytes = serde_json::to_vec_pretty(record).map_err(|error| WorkspaceError::Storage {
            operation: "serializeDiagnostic",
            path: path.clone(),
            message: error.to_string(),
        })?;
        write_bytes_atomically(&path, &bytes)
    }

    /// 按文件名顺序读取当前工作区全部诊断和修复记录。
    pub(crate) fn read_all(&self) -> WorkspaceResult<Vec<DiagnosticRecord>> {
        let directory = self.layout.diagnostics_dir();
        let entries = fs::read_dir(&directory).map_err(|error| WorkspaceError::Storage {
            operation: "readDiagnosticsDirectory",
            path: directory.clone(),
            message: error.to_string(),
        })?;
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| WorkspaceError::Storage {
                operation: "readDiagnosticEntry",
                path: directory.clone(),
                message: error.to_string(),
            })?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        paths.sort();

        paths
            .into_iter()
            .map(|path| {
                let bytes = fs::read(&path).map_err(|error| WorkspaceError::Storage {
                    operation: "readDiagnostic",
                    path: path.clone(),
                    message: error.to_string(),
                })?;
                serde_json::from_slice(&bytes).map_err(|error| WorkspaceError::Storage {
                    operation: "parseDiagnostic",
                    path,
                    message: error.to_string(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;

    use super::*;
    use crate::backend::ai_driven::ai_task_execution::workspace::{
        ArtifactDiagnostic, ArtifactId, DiagnosticSeverity, DiagnosticStage, WorkspaceManager,
    };

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn test_store(name: &str) -> (PathBuf, DiagnosticStore) {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("datavisool-diagnostic-store-{name}-{}-{sequence}", std::process::id()));
        let workspace = WorkspaceManager::new(&root)
            .create("run-1")
            .expect("workspace should be created");
        (root, DiagnosticStore::new(&workspace))
    }

    #[test]
    fn saves_records_as_independent_files_and_reads_them_back() {
        let (root, store) = test_store("append");
        let artifact_id = ArtifactId::new("result").expect("artifactId should be valid");
        let first = DiagnosticRecord::Diagnostic(ArtifactDiagnostic {
            diagnostic_id: "diagnostic-1".to_string(),
            artifact_id: Some(artifact_id),
            revision: Some(1),
            severity: DiagnosticSeverity::Error,
            stage: DiagnosticStage::Syntax,
            code: "json.missing_comma".to_string(),
            message: "missing comma".to_string(),
            details: Some(json!({ "line": 2, "column": 4 })),
            created_at: now_ms(),
        });
        let second = DiagnosticRecord::Diagnostic(ArtifactDiagnostic {
            diagnostic_id: "../../not-a-path".to_string(),
            artifact_id: None,
            revision: None,
            severity: DiagnosticSeverity::Warning,
            stage: DiagnosticStage::CrossArtifact,
            code: "id.duplicate".to_string(),
            message: "duplicate id".to_string(),
            details: None,
            created_at: now_ms(),
        });

        store.append(&first).expect("first record should be saved");
        store.append(&second).expect("second record should be saved");
        assert_eq!(store.read_all().expect("records should be readable"), vec![first, second]);
        assert_eq!(
            fs::read_dir(root.join("run-1/runtime/diagnostics"))
                .expect("diagnostics directory should exist")
                .count(),
            2
        );
        let _ = fs::remove_dir_all(root);
    }
}
