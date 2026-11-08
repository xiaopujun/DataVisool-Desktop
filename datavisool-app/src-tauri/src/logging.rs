/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Tauri 桌面端后端的日志与崩溃保护模块。
//!
//! 这个文件负责：
//! - 初始化 tracing 日志系统，优先按日志级别写入应用数据目录下的滚动日志文件
//! - 在文件日志不可用时回退到 stderr，保证后端仍然有可见日志输出
//! - 清理过期日志文件，避免日志目录无限增长
//! - 安装全局 panic hook，捕获崩溃并统一写入日志
//! - 提供异常与 backtrace 的统一记录入口，供 `main.rs` 和其他模块复用

use std::backtrace::Backtrace;
use std::fmt::Display;
use std::fs;
use std::panic::{self, PanicHookInfo};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
#[cfg(debug_assertions)]
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

/// 日志目录名，位于 Tauri 的 `AppData` 目录下。
const LOG_DIR_NAME: &str = "logs";
/// 运行日志文件名前缀，记录 INFO、WARN 与 ERROR，按天滚动生成文件。
const LOG_FILE_PREFIX: &str = "desktop-backend.log";
/// 调试日志文件名前缀，记录完整 Agent 诊断链路，按天滚动生成文件。
const DEBUG_LOG_FILE_PREFIX: &str = "desktop-backend-debug.log";
/// 保留最近的日志文件数量；由于使用 daily rolling，因此约等于保留最近 14 天。
const LOG_KEEP_FILES: usize = 14;
/// 当外部没有显式设置 `RUST_LOG` 时使用的默认过滤级别。
const DEFAULT_LOG_FILTER: &str = "info,agent.history=debug";
/// 持有文件写入器 guard，防止后台写线程过早退出。
struct FileLogGuards {
    _runtime: WorkerGuard,
    _debug: WorkerGuard,
}

/// 使用本机时区格式化日志时间戳。
#[derive(Debug, Clone, Copy)]
struct LocalLogTimer;

impl FormatTime for LocalLogTimer {
    /// 输出带本机 UTC 偏移量的 ISO 8601 时间戳。
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z"))
    }
}

/// 持有非阻塞文件写入器的 guard，防止后台写线程过早退出。
static FILE_LOG_GUARD: OnceLock<Mutex<Option<FileLogGuards>>> = OnceLock::new();
/// 标记 panic hook 是否已经安装，避免重复覆盖默认 hook。
static PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

/// 日志初始化结果，供 `main.rs` 打印当前使用的日志落盘方式。
#[derive(Debug, Clone)]
pub enum LoggingInit {
    /// 文件日志初始化成功：返回日志目录、各级别文件前缀和保留策略信息。
    File {
        log_dir: PathBuf,
        runtime_file_prefix: &'static str,
        debug_file_prefix: &'static str,
        retention_days: usize,
    },
    /// 文件日志初始化失败后退回到 stderr；`reason` 保存失败原因。
    StderrFallback { reason: String },
}

/// 初始化桌面端日志系统。
///
/// 优先尝试创建文件日志；如果文件日志初始化失败，则回退到 stderr，
/// 以确保后端仍然可以输出日志信息。
pub fn init(app: &AppHandle) -> LoggingInit {
    try_init_file_logger(app).unwrap_or_else(|err| {
        init_stderr_logger();
        LoggingInit::StderrFallback { reason: err }
    })
}

/// 尝试初始化文件日志，并返回初始化结果或失败原因。
///
/// 这个流程会先定位日志目录、创建目录、清理旧日志，再挂载按级别分流的 rolling file appender。
fn try_init_file_logger(app: &AppHandle) -> Result<LoggingInit, String> {
    let log_dir = app
        .path()
        .resolve(LOG_DIR_NAME, BaseDirectory::AppData)
        .map_err(|e| format!("resolve log dir failed: {e}"))?;
    fs::create_dir_all(&log_dir).map_err(|e| format!("create log dir failed: {e}"))?;
    cleanup_old_logs(&log_dir, LOG_FILE_PREFIX);
    cleanup_old_logs(&log_dir, DEBUG_LOG_FILE_PREFIX);

    // 使用按天滚动的文件写入器，保证运行日志和调试日志可持续归档。
    let runtime_file_appender = tracing_appender::rolling::daily(&log_dir, LOG_FILE_PREFIX);
    let (runtime_file_writer, runtime_guard) = tracing_appender::non_blocking(runtime_file_appender);
    let runtime_writer = build_writer(runtime_file_writer);
    let debug_file_appender = tracing_appender::rolling::daily(&log_dir, DEBUG_LOG_FILE_PREFIX);
    let (debug_file_writer, debug_guard) = tracing_appender::non_blocking(debug_file_appender);
    let debug_writer = build_writer(debug_file_writer);
    let env_filter = resolve_env_filter();
    let runtime_level_filter = filter_fn(|metadata| is_runtime_event(metadata.level(), metadata.target()));
    let debug_level_filter = filter_fn(|metadata| is_agent_debug_event(metadata.level(), metadata.target()));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_timer(LocalLogTimer)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_file(true)
                .with_line_number(true)
                .with_writer(runtime_writer)
                .with_filter(runtime_level_filter),
        )
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_timer(LocalLogTimer)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_file(true)
                .with_line_number(true)
                .with_writer(debug_writer)
                .with_filter(debug_level_filter),
        )
        .try_init()
        .map_err(|e| format!("init tracing subscriber failed: {e}"))?;

    let _ = FILE_LOG_GUARD.set(Mutex::new(Some(FileLogGuards { _runtime: runtime_guard, _debug: debug_guard })));

    Ok(LoggingInit::File {
        log_dir,
        runtime_file_prefix: LOG_FILE_PREFIX,
        debug_file_prefix: DEBUG_LOG_FILE_PREFIX,
        retention_days: LOG_KEEP_FILES,
    })
}

/// 判断日志是否应写入运行日志文件。
///
/// 运行日志供生产诊断使用，因此保留 INFO、WARN 和 ERROR。
fn is_runtime_level(level: &Level) -> bool {
    matches!(*level, Level::ERROR | Level::WARN | Level::INFO)
}

/// 判断运行日志是否应该保留该事件。
///
/// `RUST_LOG=debug` 时，依赖库也可能产生 INFO 日志；这些内容不属于桌面端的
/// 运行诊断，且可能重复输出请求上下文，因此不写入常规日志。
fn is_runtime_event(level: &Level, target: &str) -> bool {
    is_runtime_level(level) && !is_dependency_noise_target(target)
}

/// Debug 文件仅记录 Agent 的可观测性事件。
///
/// SQL、HTTP 和 Rig 内部调试输出会快速淹没一次任务的关键链路；Agent 代码以
/// `agent.` target 显式写入请求、响应、工具调用和运行状态。无论级别，均写入
/// debug 文件以保证失败、取消和安全审计不会缺段。
fn is_agent_debug_event(_level: &Level, target: &str) -> bool {
    target.starts_with("agent.")
}

fn is_dependency_noise_target(target: &str) -> bool {
    ["rig_core", "sqlx", "hyper", "h2", "reqwest", "tower", "tokio"]
        .iter()
        .any(|prefix| target.starts_with(prefix))
}

/// 初始化仅输出到 stderr 的回退日志配置。
///
/// 当文件日志无法创建时使用该路径，避免后端完全失去可观测性。
fn init_stderr_logger() {
    let _ = tracing_subscriber::registry()
        .with(resolve_env_filter())
        .with(
            fmt::layer()
                .with_timer(LocalLogTimer)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_file(true)
                .with_line_number(true),
        )
        .try_init();
}

/// 解析 tracing 过滤规则。
///
/// Debug 构建允许通过 `RUST_LOG` 调整日志级别；Release 构建固定使用 `info`，
/// 避免发布环境意外输出 `debug` 或 `trace` 日志。
fn resolve_env_filter() -> EnvFilter {
    #[cfg(debug_assertions)]
    {
        EnvFilter::try_from_env("RUST_LOG")
            .or_else(|_| EnvFilter::try_new(DEFAULT_LOG_FILTER))
            .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER))
    }

    #[cfg(not(debug_assertions))]
    {
        EnvFilter::new(DEFAULT_LOG_FILTER)
    }
}

/// 构造日志写入器。
///
/// Debug 构建下会同时写入 stderr，方便本地调试时即时查看；
/// Release 构建下仅写入文件，避免重复输出。
fn build_writer(file_writer: tracing_appender::non_blocking::NonBlocking) -> BoxMakeWriter {
    #[cfg(debug_assertions)]
    {
        BoxMakeWriter::new(std::io::stderr.and(file_writer))
    }

    #[cfg(not(debug_assertions))]
    {
        BoxMakeWriter::new(file_writer)
    }
}

/// 清理指定前缀的旧日志文件，只保留最近的 `LOG_KEEP_FILES` 个文件。
///
/// 这里按修改时间倒序排序，跳过目录遍历或元数据读取失败的条目，避免影响启动流程。
fn cleanup_old_logs(log_dir: &PathBuf, file_prefix: &str) {
    let entries = match fs::read_dir(log_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    let mut files = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with(file_prefix) {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((path, modified))
        })
        .collect::<Vec<_>>();

    files.sort_by(|left, right| right.1.cmp(&left.1));

    for (path, _) in files.into_iter().skip(LOG_KEEP_FILES) {
        let _ = fs::remove_file(path);
    }
}

/// 安装全局 panic hook，捕获 panic 现场并写入日志。
///
/// 该 hook 只会安装一次，且会在记录后继续调用默认 hook，保持原有输出行为。
pub fn install_panic_hook() {
    if PANIC_HOOK_INSTALLED.set(()).is_err() {
        return;
    }

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        log_panic(panic_info);
        default_hook(panic_info);
    }));
}

/// 记录一个普通异常场景，附带上下文、线程信息和 backtrace。
pub fn log_exception(context: &str, error: impl Display) {
    let current_thread = std::thread::current();
    let thread_name = current_thread.name().unwrap_or("unnamed");

    tracing::error!(
        target: "desktop_backend::exception",
        context,
        thread = %thread_name,
        error = %error,
        backtrace = %capture_backtrace(),
        "desktop backend exception"
    );
}

/// 捕获当前调用栈并格式化成字符串，供日志字段输出。
pub fn capture_backtrace() -> String {
    format!("{}", Backtrace::force_capture())
}

/// 记录 panic 事件，包括线程名、发生位置、panic payload 和 backtrace。
fn log_panic(panic_info: &PanicHookInfo<'_>) {
    let current_thread = std::thread::current();
    let thread_name = current_thread.name().unwrap_or("unnamed");
    let location = panic_info
        .location()
        .map(|location| format!("{}:{}", location.file(), location.line()))
        .unwrap_or_else(|| "unknown".to_string());

    tracing::error!(
        target: "desktop_backend::panic",
        thread = %thread_name,
        location = %location,
        payload = %panic_payload(panic_info),
        backtrace = %capture_backtrace(),
        "desktop backend panic captured"
    );
}

/// 将 panic payload 转换为可读字符串，优先保留字符串内容。
fn panic_payload(panic_info: &PanicHookInfo<'_>) -> String {
    if let Some(payload) = panic_info.payload().downcast_ref::<&str>() {
        (*payload).to_string()
    } else if let Some(payload) = panic_info.payload().downcast_ref::<String>() {
        payload.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{is_agent_debug_event, is_runtime_event, is_runtime_level};
    use tracing::Level;

    #[test]
    fn runtime_levels_exclude_debug_levels() {
        assert!(is_runtime_level(&Level::ERROR));
        assert!(is_runtime_level(&Level::WARN));
        assert!(is_runtime_level(&Level::INFO));
        assert!(!is_runtime_level(&Level::DEBUG));
        assert!(!is_runtime_level(&Level::TRACE));
    }

    #[test]
    fn debug_file_only_keeps_agent_events() {
        assert!(is_agent_debug_event(&Level::DEBUG, "agent.model"));
        assert!(is_agent_debug_event(&Level::TRACE, "agent.tool"));
        assert!(is_agent_debug_event(&Level::INFO, "agent.security.audit"));
        assert!(is_agent_debug_event(&Level::ERROR, "agent.run"));
        assert!(!is_agent_debug_event(&Level::DEBUG, "sqlx::query"));
    }

    #[test]
    fn runtime_file_excludes_dependency_noise() {
        assert!(is_runtime_event(&Level::INFO, "desktop_backend::task"));
        assert!(!is_runtime_event(&Level::INFO, "rig_core::agent"));
        assert!(!is_runtime_event(&Level::INFO, "reqwest::connect"));
    }
}
