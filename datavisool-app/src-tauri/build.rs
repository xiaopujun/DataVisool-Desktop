/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Tauri 桌面端构建脚本。
//!
//! 负责嵌入 Agent 组件资料、声明图标资源的变更监听，并委托 `tauri_build` 生成 Tauri 构建期元数据。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "src/backend/ai_driven/component_docs/g2plot_fragments.rs"]
mod g2plot_fragments;

const COMPONENT_DOCS_DIR: &str = "resources/ai/component-optimization";
const G2PLOT_COMMON_FRAGMENTS_PATH: &str = "resources/ai/G2Plot公共配置分块说明.md";
const COMPONENT_DEFINITIONS_DIR: &str = "../src/comps";
const COMPONENT_CATALOG_TOOL: &str = "../src/frontend-effect/tools/GetComponentCatalogTool.ts";
const REQUIRED_HEADINGS: [&str; 5] = ["组件信息", "数据格式要求", "可配置项", "校验规则", "注意事项"];
const COMPONENT_SPEC_OUTPUT_BUDGET_BYTES: u64 = 256 * 1024;

/// 构建脚本入口。
fn main() {
    verify_ripgrep_sidecars();
    generate_component_docs_manifest();
    // 图标资源变化时强制重新运行 build script，确保安装包图标同步更新。
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/icon.icns");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon-square.png");

    tauri_build::build()
}

fn generate_component_docs_manifest() {
    println!("cargo:rerun-if-changed={COMPONENT_DOCS_DIR}");
    println!("cargo:rerun-if-changed={G2PLOT_COMMON_FRAGMENTS_PATH}");
    let common_fragments_source = fs::read_to_string(G2PLOT_COMMON_FRAGMENTS_PATH)
        .unwrap_or_else(|error| panic!("读取 G2Plot 公共配置片段失败 {G2PLOT_COMMON_FRAGMENTS_PATH}：{error}"));
    let common_fragments = g2plot_fragments::parse_fragments(&common_fragments_source)
        .unwrap_or_else(|error| panic!("G2Plot 公共配置片段格式无效：{error}"));
    let mut entries = fs::read_dir(COMPONENT_DOCS_DIR)
        .unwrap_or_else(|error| panic!("读取组件 AI 资料目录失败：{error}"))
        .map(|entry| entry.unwrap_or_else(|error| panic!("读取组件 AI 资料项失败：{error}")))
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "md"))
        .map(|entry| validate_component_doc(&entry.path(), &common_fragments))
        .collect::<Vec<_>>();
    entries.sort_unstable_by(|left, right| left.component_type.cmp(&right.component_type));
    if entries.is_empty() {
        panic!("组件 AI 资料目录不能为空");
    }
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry.byte_size > COMPONENT_SPEC_OUTPUT_BUDGET_BYTES)
    {
        panic!(
            "组件 {} 的源文档为 {} 字节，单组件已超过完整规格预算 {} 字节",
            entry.component_type, entry.byte_size, COMPONENT_SPEC_OUTPUT_BUDGET_BYTES
        );
    }
    validate_component_doc_coverage(&entries);

    let mut corpus_hasher = Sha256::new();
    corpus_hasher.update(b"g2plot-common-fragments\0");
    corpus_hasher.update(common_fragments_source.as_bytes());
    corpus_hasher.update([0]);
    let mut generated = String::from("pub(crate) const EMBEDDED_COMPONENT_DOCS: &[(&str, &str, u64, &str)] = &[\n");
    for entry in &entries {
        corpus_hasher.update(entry.component_type.as_bytes());
        corpus_hasher.update([0]);
        corpus_hasher.update(entry.sha256.as_bytes());
        corpus_hasher.update([0]);
        writeln!(
            generated,
            "    (\"{}\", include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{COMPONENT_DOCS_DIR}/{}.md\")), {}, \"{}\"),",
            entry.component_type, entry.component_type, entry.byte_size, entry.sha256
        )
        .expect("生成组件 AI 资料索引失败");
    }
    generated.push_str("];\n");
    writeln!(
        generated,
        "pub(crate) const EMBEDDED_G2PLOT_COMMON_FRAGMENTS: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{G2PLOT_COMMON_FRAGMENTS_PATH}\"));"
    )
    .expect("生成 G2Plot 公共配置片段来源失败");
    writeln!(generated, "pub(crate) const COMPONENT_DOCS_CORPUS_VERSION: &str = \"{:x}\";", corpus_hasher.finalize())
        .expect("生成组件 AI 资料版本失败");

    let output = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR 未设置")).join("component_optimization_docs.rs");
    fs::write(output, generated).expect("写入组件 AI 资料索引失败");
}

struct ComponentDocBuildEntry {
    component_type: String,
    byte_size: u64,
    sha256: String,
}

fn validate_component_doc(path: &Path, common_fragments: &BTreeMap<String, Value>) -> ComponentDocBuildEntry {
    println!("cargo:rerun-if-changed={}", path.display());
    let file_name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| panic!("组件 AI 资料文件名无效：{}", path.display()));
    if !file_name.chars().all(|character| character.is_ascii_alphanumeric()) {
        panic!("组件 AI 资料文件名只能使用组件类型：{}", path.display());
    }
    let content = fs::read_to_string(path).unwrap_or_else(|error| panic!("读取组件 AI 资料失败 {}：{error}", path.display()));
    if content.lines().any(|line| line.chars().count() > 2_000) {
        panic!("组件 AI 资料包含超过 2000 字符的单行，无法安全分页：{}", path.display());
    }
    let component_type = extract_component_type(&content).unwrap_or_else(|| panic!("组件 AI 资料缺少 componentType：{}", path.display()));
    if component_type != file_name {
        panic!("组件 AI 资料文件名与 componentType 不一致：{file_name} != {component_type}");
    }
    for heading in REQUIRED_HEADINGS {
        if !content.lines().any(|line| line.trim() == format!("## {heading}")) {
            panic!("组件 AI 资料缺少“{heading}”章节：{}", path.display());
        }
    }
    validate_complete_spec_sections(path, &content, common_fragments);
    ComponentDocBuildEntry {
        component_type: component_type.to_string(),
        byte_size: u64::try_from(content.len()).expect("组件 AI 资料大小溢出"),
        sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
    }
}

fn validate_complete_spec_sections(path: &Path, content: &str, common_fragments: &BTreeMap<String, Value>) {
    let configurable = extract_level_two_section(content, "可配置项").unwrap_or_else(|| panic!("组件 AI 资料缺少可配置项：{}", path.display()));
    let configurable = first_json_block(configurable).unwrap_or_else(|| panic!("组件 AI 资料可配置项缺少有效 JSON：{}", path.display()));
    if !configurable
        .get("style")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|style| !style.is_empty())
    {
        panic!("组件 AI 资料可配置项必须包含非空 style：{}", path.display());
    }
    let references = g2plot_fragments::collect_style_references(&configurable)
        .unwrap_or_else(|error| panic!("组件 AI 资料 G2Plot 标签无效 {}：{error}", path.display()));
    let missing = references
        .iter()
        .filter(|tag| !common_fragments.contains_key(*tag))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        panic!("组件 AI 资料引用了未定义的 G2Plot 公共配置片段 {}：{missing:?}", path.display());
    }
    let validation = json_blocks(content)
        .into_iter()
        .find(|value| value.get("schemaVersion").and_then(serde_json::Value::as_str) == Some("component-style-doc/v1"))
        .unwrap_or_else(|| panic!("组件 AI 资料缺少 component-style-doc/v1：{}", path.display()));
    if !validation
        .get("allowedStylePaths")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|paths| !paths.is_empty())
    {
        panic!("组件 AI 资料 allowedStylePaths 不能为空：{}", path.display());
    }
    let requirements = extract_level_two_section(content, "数据格式要求").unwrap_or_default();
    let supports = requirements.matches("该组件支持通过 `dataPatch` 修改数据。").count();
    let unsupported = requirements.matches("该组件不支持通过 `dataPatch` 修改数据。").count();
    if supports + unsupported != 1 {
        panic!("组件 AI 资料必须明确声明一次数据能力：{}", path.display());
    }
    if unsupported == 1 {
        let notes = extract_level_two_section(content, "注意事项").unwrap_or_default();
        if !json_blocks(notes)
            .iter()
            .any(|value| value.get("stylePatch").is_some() || value.get("config").is_some())
        {
            panic!("无数据组件必须包含可解析的最小样式示例：{}", path.display());
        }
    }
}

fn validate_component_doc_coverage(entries: &[ComponentDocBuildEntry]) {
    println!("cargo:rerun-if-changed={COMPONENT_DEFINITIONS_DIR}");
    println!("cargo:rerun-if-changed={COMPONENT_CATALOG_TOOL}");
    let registered = registered_component_types(Path::new(COMPONENT_DEFINITIONS_DIR));
    let catalog_tool = fs::read_to_string(COMPONENT_CATALOG_TOOL).unwrap_or_else(|error| panic!("读取 AI 组件目录工具失败：{error}"));
    let excluded = extract_excluded_component_types(&catalog_tool);
    let expected = registered.difference(&excluded).cloned().collect::<BTreeSet<_>>();
    let documented = entries
        .iter()
        .map(|entry| entry.component_type.clone())
        .collect::<BTreeSet<_>>();
    let missing = expected.difference(&documented).cloned().collect::<Vec<_>>();
    let extra = documented.difference(&expected).cloned().collect::<Vec<_>>();
    if !missing.is_empty() || !extra.is_empty() {
        panic!("AI 开放组件与规格文档不一致；缺失：{missing:?}；多余：{extra:?}");
    }
}

fn registered_component_types(root: &Path) -> BTreeSet<String> {
    let mut files = Vec::new();
    collect_definition_files(root, &mut files);
    files
        .into_iter()
        .filter_map(|path| {
            let content = fs::read_to_string(&path).unwrap_or_else(|error| panic!("读取组件定义失败 {}：{error}", path.display()));
            match extract_quoted_value_after(&content, "compKey:") {
                Some(component_type) => Some(component_type.to_string()),
                None if is_abstract_definition(&content) => None,
                None => panic!("非抽象组件定义缺少可解析 compKey：{}", path.display()),
            }
        })
        .collect()
}

fn is_abstract_definition(content: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("abstract class ") || line.starts_with("export abstract class ")
    })
}

fn collect_definition_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap_or_else(|error| panic!("读取组件定义目录失败 {}：{error}", directory.display())) {
        let path = entry.unwrap_or_else(|error| panic!("读取组件定义项失败：{error}")).path();
        if path.is_dir() {
            collect_definition_files(&path, files);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("Definition.ts"))
        {
            println!("cargo:rerun-if-changed={}", path.display());
            files.push(path);
        }
    }
}

fn extract_excluded_component_types(content: &str) -> BTreeSet<String> {
    let (_, tail) = content
        .split_once("const AI_DRIVEN_EXCLUDED_COMPONENT_TYPES")
        .unwrap_or_else(|| panic!("AI 组件目录工具缺少排除清单"));
    let list = tail
        .split_once('[')
        .and_then(|(_, tail)| tail.split_once(']').map(|(list, _)| list))
        .unwrap_or_else(|| panic!("AI 组件排除清单格式无效"));
    list.lines()
        .filter_map(|line| extract_first_quoted(line))
        .map(str::to_string)
        .collect()
}

fn extract_quoted_value_after<'a>(content: &'a str, marker: &str) -> Option<&'a str> {
    let (_, tail) = content.split_once(marker)?;
    extract_first_quoted(tail)
}

fn extract_first_quoted(content: &str) -> Option<&str> {
    let start = content.char_indices().find(|(_, character)| matches!(character, '\'' | '"'))?;
    let quote = start.1;
    let tail = &content[start.0 + quote.len_utf8()..];
    let end = tail.find(quote)?;
    Some(&tail[..end])
}

fn extract_level_two_section<'a>(content: &'a str, heading: &str) -> Option<&'a str> {
    let marker = format!("## {heading}");
    let (_, tail) = content.split_once(&marker)?;
    Some(tail.split("\n## ").next().unwrap_or(tail).trim())
}

fn first_json_block(content: &str) -> Option<serde_json::Value> {
    json_blocks(content).into_iter().next()
}

fn json_blocks(content: &str) -> Vec<serde_json::Value> {
    content
        .split("```json")
        .skip(1)
        .filter_map(|tail| tail.split_once("```").map(|(json, _)| json.trim()))
        .filter_map(|json| serde_json::from_str(json).ok())
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RipgrepManifest {
    version: String,
    targets: Vec<RipgrepTarget>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RipgrepTarget {
    binary: String,
    binary_sha256: String,
}

fn verify_ripgrep_sidecars() {
    const MANIFEST_PATH: &str = "binaries/ripgrep-manifest.json";
    println!("cargo:rerun-if-changed={MANIFEST_PATH}");
    let manifest: RipgrepManifest =
        serde_json::from_slice(&fs::read(MANIFEST_PATH).unwrap_or_else(|error| panic!("读取 ripgrep Sidecar 清单失败：{error}")))
            .unwrap_or_else(|error| panic!("解析 ripgrep Sidecar 清单失败：{error}"));
    if manifest.version.trim().is_empty() || manifest.targets.is_empty() {
        panic!("ripgrep Sidecar 清单不能为空");
    }
    for target in manifest.targets {
        let path = Path::new("binaries").join(&target.binary);
        println!("cargo:rerun-if-changed={}", path.display());
        let metadata =
            fs::symlink_metadata(&path).unwrap_or_else(|_| panic!("缺少 ripgrep Sidecar：{}；请运行 pnpm run sidecar:prepare", path.display()));
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            panic!("ripgrep Sidecar 必须是普通文件：{}", path.display());
        }
        let digest = format!(
            "{:x}",
            Sha256::digest(fs::read(&path).unwrap_or_else(|error| panic!("读取 ripgrep Sidecar 失败 {}：{error}", path.display())))
        );
        if digest != target.binary_sha256 {
            panic!("ripgrep Sidecar SHA-256 校验失败：{}", path.display());
        }
    }
}

fn extract_component_type(content: &str) -> Option<&str> {
    let (_, rest) = content.split_once("\"componentType\"")?;
    let (_, value) = rest.split_once(':')?;
    let value = value.trim_start().strip_prefix('"')?;
    value.split_once('"').map(|(component_type, _)| component_type)
}
