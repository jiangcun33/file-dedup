//! 文件去重 - Tauri 应用入口与命令桥接层

mod commands;

use commands::*;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// 扫描状态：保存当前扫描的取消标志
pub struct ScanState {
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

impl Default for ScanState {
    fn default() -> Self {
        Self {
            cancel: Mutex::new(None),
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(ScanState::default())
        .invoke_handler(tauri::generate_handler![
            run_scan,
            cancel_scan,
            apply_action,
            clear_cache,
            get_cache_stats,
            default_cache_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 供命令模块使用的便捷函数：应用数据目录
pub fn app_data_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("无法获取数据目录: {e}"))
}
