/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri 桌面端应用入口。
//!
//! 这里负责注册插件、初始化日志/数据库/应用状态、创建主窗口，并把前端可调用的
//! Rust Command 挂到 Tauri 运行时上。业务路由本身下沉到 `backend` 模块。

#[cfg(all(feature = "desktop-e2e", not(debug_assertions)))]
compile_error!("desktop-e2e is restricted to debug builds");

mod app_state;
mod backend;
mod db;
mod logging;

use tauri::{Manager, WebviewWindow, WebviewWindowBuilder};

/// macOS WKWebView 的兼容 UA。
///
/// 部分页面能力依赖 Safari 风格 UA，这里只在 macOS 主窗口创建前覆写。
const MACOS_WEBVIEW_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 11_0_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/14.0 Safari/605.1.15";

/// 调试命令：允许前端在内部开发构建里主动打开当前窗口 DevTools。
#[tauri::command]
fn datavisool_open_devtools(window: WebviewWindow) {
    window.open_devtools();
}

/// 启动 Tauri 应用并完成桌面端后端的运行时装配。
fn main() {
    let builder = tauri::Builder::default();
    #[cfg(feature = "desktop-e2e")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());

    let run_result = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // 日志必须先于数据库和窗口初始化，后续启动失败才能留下诊断信息。
            match logging::init(&handle) {
                logging::LoggingInit::File { log_dir, runtime_file_prefix, debug_file_prefix, retention_days } => {
                    tracing::info!(
                        "desktop backend file logging enabled: dir={}, runtime_prefix={}, debug_prefix={}, retention_days={}",
                        log_dir.display(),
                        runtime_file_prefix,
                        debug_file_prefix,
                        retention_days,
                    );
                }
                logging::LoggingInit::StderrFallback { reason } => {
                    tracing::warn!("desktop backend file logging unavailable, fallback to stderr: {}", reason);
                }
            }

            logging::install_panic_hook();

            // AppState 统一维护 AppData/Home/资源存储目录等路径派生结果。
            let app_state = app_state::AppState::init(&handle).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            let db_file = app_state.db_file().to_path_buf();
            // SQLite 初始化包含迁移执行，必须同步完成后才能注册后端 Command。
            let db = tauri::async_runtime::block_on(async move {
                db::DbState::init(&db_file)
                    .await
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            })
            .map_err(|e| -> Box<dyn std::error::Error> {
                logging::log_exception("desktop backend database initialization failed", &e);
                Box::new(e)
            })?;
            // 数据库里的持久化配置需要回灌到 AppState，保证资源路径从启动起生效。
            let app_config = tauri::async_runtime::block_on(db.load_app_config())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("load app config failed: {e}")))?;
            app_state
                .apply_app_config(&app_config)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            match tauri::async_runtime::block_on(backend::cleanup_expired_agent_attachments(&db.pool, &app_state.resource_storage_root())) {
                Ok(outcome) if outcome.deleted > 0 || outcome.failed > 0 => {
                    tracing::info!(deleted = outcome.deleted, failed = outcome.failed, "Agent 临时附件过期清理完成");
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "Agent 临时附件过期清理失败"),
            }

            // 将初始化好的数据库状态（DbState）注册到 Tauri 的全局状态管理容器中
            // 注册后，后续的 Command（如 datavisool_http_request）就可以通过 State<'_, DbState> 参数获取到该对象
            handle.manage(db);
            handle.manage(app_state);
            handle.manage(backend::HttpRequestCancelRegistry::default());
            handle.manage(backend::FrontendEffectRegistry::default());

            // 主窗口由 tauri.conf.json 提供基础配置，这里只补运行时差异项。
            let mut main_window_config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "missing main window config"))?;
            #[cfg(target_os = "macos")]
            {
                main_window_config.user_agent = Some(MACOS_WEBVIEW_USER_AGENT.to_string());
            }
            #[cfg(target_os = "windows")]
            {
                // The native shadow leaves a square border on older Windows versions.
                main_window_config.shadow = false;
            }
            let main_window = WebviewWindowBuilder::from_config(app.handle(), &main_window_config)?.build()?;

            #[cfg(feature = "internal-devtools")]
            {
                tracing::info!("internal-devtools 已启用，启动时自动打开 DevTools");
                main_window.open_devtools();
            }
            #[cfg(not(feature = "internal-devtools"))]
            {
                let _ = &main_window;
            }

            Ok(())
        })
        // 注册前端可调用的 Rust 命令（Command）
        // 这里注册了 datavisool_http_request，前端可以用 invoke('datavisool_http_request', { ... }) 调用它
        .invoke_handler(tauri::generate_handler![
            datavisool_open_devtools,
            backend::datavisool_http_request,
            backend::datavisool_register_http_request,
            backend::datavisool_cancel_http_request,
            backend::datavisool_unregister_http_request,
            backend::frontend_effect::datavisool_complete_frontend_effect
        ])
        // 读取 tauri.conf.json 配置并启动应用主循环
        .run(tauri::generate_context!());

    if let Err(err) = run_result {
        logging::log_exception("desktop backend tauri application run failed", &err);
        std::process::exit(1);
    }
}
