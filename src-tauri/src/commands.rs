//! Tauri 命令：前端通过 invoke 调用

use crate::ScanState;
use dedup_core::{ActionResult, BatchActionRequest, ProgressUpdate, ScanOptions, ScanResult};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

/// 执行一次扫描。期间通过 `scan-progress` 事件推送进度；返回最终分组结果。
#[tauri::command]
pub async fn run_scan(
    app: AppHandle,
    state: State<'_, ScanState>,
    options: ScanOptions,
) -> Result<ScanResult, String> {
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    *state.cancel.lock().unwrap() = Some(cancel.clone());

    let (tx, rx) = crossbeam_channel::unbounded::<ProgressUpdate>();
    let app2 = app.clone();
    let forwarder = std::thread::spawn(move || {
        while let Ok(p) = rx.recv() {
            let _ = app2.emit("scan-progress", p);
        }
    });

    let tx_for_scan = tx.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        dedup_core::run_scan(&options, Some(&tx_for_scan), Some(&cancel))
    })
    .await
    .map_err(|e| e.to_string())?;

    drop(tx);
    let _ = forwarder.join();
    *state.cancel.lock().unwrap() = None;
    result
}

/// 请求取消当前扫描（置位取消标志，扫描尽快停止）
#[tauri::command]
pub fn cancel_scan(state: State<'_, ScanState>) {
    if let Some(c) = state.cancel.lock().unwrap().as_ref() {
        c.store(true, Ordering::Relaxed);
    }
}

/// 批量执行操作（回收站/删除/硬链接/移动/复制），支持跨组文件
#[tauri::command]
pub fn apply_action(req: BatchActionRequest) -> Vec<ActionResult> {
    dedup_core::action::apply_batch_action(&req)
}

/// 删除空文件夹（含安全校验）
#[tauri::command]
pub fn remove_empty_dirs(paths: Vec<String>) -> Vec<ActionResult> {
    dedup_core::action::remove_empty_dirs(&paths)
}

/// 清空哈希缓存，返回删除的条目数
#[tauri::command]
pub fn clear_cache(path: String) -> Result<u64, String> {
    let mut c = dedup_core::cache::HashCache::open(&path).map_err(|e| e.to_string())?;
    c.clear().map_err(|e| e.to_string())
}

/// 查询哈希缓存条目数
#[tauri::command]
pub fn get_cache_stats(path: String) -> Result<serde_json::Value, String> {
    let c = dedup_core::cache::HashCache::open(&path).map_err(|e| e.to_string())?;
    let n = c.count().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "entries": n }))
}

/// 应用版本号（来自 Cargo 包版本）
#[tauri::command]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 默认缓存文件路径（应用数据目录）
#[tauri::command]
pub fn default_cache_path(app: AppHandle) -> Result<String, String> {
    let dir = crate::app_data_dir(&app)?;
    Ok(dir.join("cache.db").to_string_lossy().into_owned())
}
