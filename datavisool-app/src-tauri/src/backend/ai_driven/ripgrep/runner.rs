/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Tauri Sidecar 启动、流式读取、取消与进程回收。

use std::future::pending;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

use tauri_plugin_shell::process::{Command, CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::watch;

use super::parser::RipgrepLineParser;
use super::types::{RipgrepLimits, RipgrepSearchOptions, SearchCaseMode, SearchMode, SearchTruncationReason};
use super::{RipgrepError, RipgrepResult, SearchCorpus};

pub(super) enum RipgrepCommandKind<'a> {
    Files,
    Content(&'a RipgrepSearchOptions),
}

pub(super) struct RipgrepRunRequest<'a> {
    pub kind: RipgrepCommandKind<'a>,
    pub globs: &'a [String],
    pub include_hidden: bool,
    pub respect_ignore_files: bool,
    pub raw_output_budget: usize,
}

pub(super) struct RipgrepRunResult {
    pub raw_output_bytes: usize,
    pub truncation_reason: Option<SearchTruncationReason>,
}

#[derive(Clone)]
pub(super) struct RipgrepRunner {
    program: RunnerProgram,
    limits: RipgrepLimits,
}

#[derive(Clone)]
enum RunnerProgram {
    Sidecar(tauri::AppHandle),
    #[cfg(test)]
    Direct {
        path: PathBuf,
        command: Arc<dyn Fn() -> Command + Send + Sync>,
    },
}

impl RipgrepRunner {
    pub(super) fn new(app_handle: tauri::AppHandle, limits: RipgrepLimits) -> Self {
        Self { program: RunnerProgram::Sidecar(app_handle), limits }
    }

    #[cfg(test)]
    pub(super) fn direct<R: tauri::Runtime>(app_handle: tauri::AppHandle<R>, binary: PathBuf, limits: RipgrepLimits) -> Self {
        let command_path = binary.clone();
        let command = Arc::new(move || app_handle.shell().command(&command_path));
        Self { program: RunnerProgram::Direct { path: binary, command }, limits }
    }

    pub(super) async fn run<P: RipgrepLineParser>(
        &self,
        corpus: &SearchCorpus,
        request: RipgrepRunRequest<'_>,
        cancel_rx: watch::Receiver<bool>,
        parser: &mut P,
    ) -> RipgrepResult<RipgrepRunResult> {
        if *cancel_rx.borrow() {
            return Err(RipgrepError::cancelled());
        }
        self.ensure_program_exists()?;
        let args = build_args(&request.kind, request.globs, request.include_hidden, request.respect_ignore_files);
        let command = self
            .command()
            .map_err(|_| RipgrepError::spawn_failed("无法创建 rg Sidecar 命令"))?
            .args(args)
            .env_clear()
            .current_dir(corpus.root())
            .set_raw_out(true);
        let (mut receiver, child) = command
            .spawn()
            .map_err(|error| RipgrepError::spawn_failed(format!("rg Sidecar 启动失败：{}", bounded(&error.to_string(), 500))))?;
        let mut child = ChildGuard::new(child);
        let mut cancellation = Box::pin(wait_for_cancel(cancel_rx));
        let deadline = tokio::time::sleep(self.limits.timeout);
        tokio::pin!(deadline);
        let mut stdout_buffer = Vec::new();
        let mut stderr = Vec::new();
        let mut raw_output_bytes = 0usize;

        loop {
            tokio::select! {
                event = receiver.recv() => {
                    let Some(event) = event else {
                        child.kill();
                        return Err(RipgrepError::process_failed("rg Sidecar 输出通道提前关闭"));
                    };
                    match event {
                        CommandEvent::Stdout(chunk) => {
                            raw_output_bytes = raw_output_bytes.saturating_add(chunk.len());
                            if raw_output_bytes > request.raw_output_budget {
                                child.kill();
                                wait_for_termination(&mut receiver).await;
                                return Ok(RipgrepRunResult {
                                    raw_output_bytes: request.raw_output_budget,
                                    truncation_reason: Some(SearchTruncationReason::RawOutputLimit),
                                });
                            }
                            stdout_buffer.extend_from_slice(&chunk);
                            let outcome = match consume_lines(&mut stdout_buffer, self.limits.max_line_bytes, parser) {
                                Ok(outcome) => outcome,
                                Err(error) => {
                                    child.kill();
                                    wait_for_termination(&mut receiver).await;
                                    return Err(error);
                                }
                            };
                            match outcome {
                                ConsumeOutcome::Continue => {}
                                ConsumeOutcome::ParserStopped => {
                                    child.kill();
                                    wait_for_termination(&mut receiver).await;
                                    return Ok(RipgrepRunResult {
                                        raw_output_bytes,
                                        truncation_reason: None,
                                    });
                                }
                                ConsumeOutcome::LineLimit => {
                                    child.kill();
                                    wait_for_termination(&mut receiver).await;
                                    return Ok(RipgrepRunResult {
                                        raw_output_bytes,
                                        truncation_reason: Some(SearchTruncationReason::LineLimit),
                                    });
                                }
                            }
                        }
                        CommandEvent::Stderr(chunk) => {
                            raw_output_bytes = raw_output_bytes.saturating_add(chunk.len());
                            if raw_output_bytes > request.raw_output_budget {
                                child.kill();
                                wait_for_termination(&mut receiver).await;
                                return Ok(RipgrepRunResult {
                                    raw_output_bytes: request.raw_output_budget,
                                    truncation_reason: Some(SearchTruncationReason::RawOutputLimit),
                                });
                            }
                            let remaining = self.limits.max_stderr_bytes.saturating_sub(stderr.len());
                            stderr.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                        }
                        CommandEvent::Error(error) => {
                            child.kill();
                            wait_for_termination(&mut receiver).await;
                            return Err(RipgrepError::process_failed(format!("rg Sidecar 读取失败：{}", bounded(&error, 500))));
                        }
                        CommandEvent::Terminated(payload) => {
                            child.disarm();
                            if !stdout_buffer.is_empty() {
                                if stdout_buffer.len() > self.limits.max_line_bytes {
                                    return Ok(RipgrepRunResult {
                                        raw_output_bytes,
                                        truncation_reason: Some(SearchTruncationReason::LineLimit),
                                    });
                                }
                                if parser.push_line(strip_carriage_return(&stdout_buffer))? {
                                    return Ok(RipgrepRunResult {
                                        raw_output_bytes,
                                        truncation_reason: None,
                                    });
                                }
                            }
                            return finish_process(payload.code, raw_output_bytes, stderr, corpus.root());
                        }
                        _ => {}
                    }
                }
                _ = &mut cancellation => {
                    child.kill();
                    wait_for_termination(&mut receiver).await;
                    return Err(RipgrepError::cancelled());
                }
                _ = &mut deadline => {
                    child.kill();
                    wait_for_termination(&mut receiver).await;
                    return Err(RipgrepError::timeout());
                }
            }
        }
    }

    fn command(&self) -> Result<Command, tauri_plugin_shell::Error> {
        match &self.program {
            RunnerProgram::Sidecar(app_handle) => app_handle.shell().sidecar("rg"),
            #[cfg(test)]
            RunnerProgram::Direct { command, .. } => Ok(command()),
        }
    }

    fn ensure_program_exists(&self) -> RipgrepResult<()> {
        let path = match &self.program {
            RunnerProgram::Sidecar(_) => sidecar_path().map_err(|_| RipgrepError::binary_missing())?,
            #[cfg(test)]
            RunnerProgram::Direct { path, .. } => path.clone(),
        };
        let metadata = std::fs::symlink_metadata(path).map_err(|_| RipgrepError::binary_missing())?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(RipgrepError::binary_missing());
        }
        Ok(())
    }
}

fn build_args(kind: &RipgrepCommandKind<'_>, globs: &[String], include_hidden: bool, respect_ignore_files: bool) -> Vec<String> {
    let mut args = match kind {
        RipgrepCommandKind::Files => vec!["--files".to_string()],
        RipgrepCommandKind::Content(_) => vec!["--json".to_string()],
    };
    args.extend(["--no-config", "--no-follow", "--sort=path"].into_iter().map(str::to_string));
    if include_hidden {
        args.push("--hidden".to_string());
    }
    if respect_ignore_files {
        // Workspace 可能尚未初始化为 Git 仓库；仍应遵守其中的 .gitignore。
        args.push("--no-require-git".to_string());
    } else {
        args.push("--no-ignore".to_string());
    }
    for glob in globs {
        args.push("--glob".to_string());
        args.push(glob.clone());
    }
    if let RipgrepCommandKind::Content(options) = kind {
        if options.mode == SearchMode::Literal {
            args.push("--fixed-strings".to_string());
        }
        args.push(
            match options.case_mode {
                SearchCaseMode::Sensitive => "--case-sensitive",
                SearchCaseMode::Insensitive => "--ignore-case",
                SearchCaseMode::Smart => "--smart-case",
            }
            .to_string(),
        );
        args.push("--regexp".to_string());
        args.push(options.query.clone());
    }
    args.push(".".to_string());
    args
}

enum ConsumeOutcome {
    Continue,
    ParserStopped,
    LineLimit,
}

fn consume_lines<P: RipgrepLineParser>(buffer: &mut Vec<u8>, max_line_bytes: usize, parser: &mut P) -> RipgrepResult<ConsumeOutcome> {
    while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
        if newline > max_line_bytes {
            return Ok(ConsumeOutcome::LineLimit);
        }
        let remainder = buffer.split_off(newline + 1);
        buffer.truncate(newline);
        let stopped = parser.push_line(strip_carriage_return(buffer))?;
        *buffer = remainder;
        if stopped {
            return Ok(ConsumeOutcome::ParserStopped);
        }
    }
    if buffer.len() > max_line_bytes {
        Ok(ConsumeOutcome::LineLimit)
    } else {
        Ok(ConsumeOutcome::Continue)
    }
}

fn strip_carriage_return(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn finish_process(code: Option<i32>, raw_output_bytes: usize, stderr: Vec<u8>, root: &Path) -> RipgrepResult<RipgrepRunResult> {
    if matches!(code, Some(0 | 1)) {
        return Ok(RipgrepRunResult { raw_output_bytes, truncation_reason: None });
    }
    let mut message = String::from_utf8_lossy(&stderr).into_owned();
    if let Some(root) = root.to_str() {
        message = message.replace(root, "<search-root>");
    }
    let message = bounded(message.trim(), 2_000);
    Err(RipgrepError::process_failed(if message.is_empty() {
        format!("rg Sidecar 异常退出：code={code:?}")
    } else {
        format!("rg Sidecar 异常退出：code={code:?}, stderr={message}")
    }))
}

fn sidecar_path() -> std::io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let file_name = if cfg!(windows) { "rg.exe" } else { "rg" };
    Ok(executable.parent().unwrap_or(Path::new(".")).join(file_name))
}

async fn wait_for_cancel(mut receiver: watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    while receiver.changed().await.is_ok() {
        if *receiver.borrow() {
            return;
        }
    }
    pending::<()>().await;
}

async fn wait_for_termination(receiver: &mut tauri::async_runtime::Receiver<CommandEvent>) {
    let wait = async {
        while let Some(event) = receiver.recv().await {
            if matches!(event, CommandEvent::Terminated(_)) {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), wait).await;
}

struct ChildGuard(Option<CommandChild>);

impl ChildGuard {
    fn new(child: CommandChild) -> Self {
        Self(Some(child))
    }

    fn kill(&mut self) {
        if let Some(child) = self.0.take() {
            let _ = child.kill();
        }
    }

    fn disarm(&mut self) {
        self.0.take();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(query: &str) -> RipgrepSearchOptions {
        RipgrepSearchOptions {
            query: query.to_string(),
            mode: SearchMode::Literal,
            case_mode: SearchCaseMode::Sensitive,
            globs: vec!["*.rs".to_string()],
            include_hidden: true,
            respect_ignore_files: false,
            max_results: 50,
        }
    }

    #[test]
    fn query_is_always_a_value_after_regexp() {
        let options = options("--pre=evil");
        let args = build_args(&RipgrepCommandKind::Content(&options), &options.globs, true, false);
        let index = args.iter().position(|argument| argument == "--regexp").unwrap();
        assert_eq!(args[index + 1], "--pre=evil");
        assert_eq!(args.last().map(String::as_str), Some("."));
        assert!(!args.iter().any(|argument| argument == "--pre" || argument == "--follow"));
    }

    #[test]
    fn line_consumer_rejects_unbounded_lines_before_parsing() {
        struct Never;
        impl RipgrepLineParser for Never {
            fn push_line(&mut self, _line: &[u8]) -> RipgrepResult<bool> {
                panic!("oversized line must not reach parser")
            }
        }
        let mut buffer = vec![b'x'; 11];
        buffer.push(b'\n');
        assert!(matches!(consume_lines(&mut buffer, 10, &mut Never).unwrap(), ConsumeOutcome::LineLimit));
    }
}
