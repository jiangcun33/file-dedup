//! 文件去重核心引擎（无 UI 依赖）
//!
//! 模块划分：
//! - [`models`]  数据结构定义
//! - [`scan`]    目录遍历与文件收集
//! - [`hash`]    三级漏斗哈希与重复分组
//! - [`cache`]   SQLite 哈希缓存
//! - [`action`]  文件操作（回收站/删除/硬链接/移动/复制）
//! - [`progress`] 进度与取消

pub mod action;
pub mod cache;
pub mod hash;
pub mod models;
pub mod progress;
pub mod scan;

pub use models::{ActionItem, ActionKind, ActionRequest, ActionResult, BatchActionRequest, DuplicateGroup, FileEntry, KeepStrategy, ProgressUpdate, ScanOptions, ScanResult};

/// 执行一次完整扫描，返回重复分组结果。
/// `progress_tx` 可选：向调用方推送进度；`cancel` 可选：置位后尽快中止。
pub fn run_scan(
    opts: &ScanOptions,
    progress_tx: Option<&crossbeam_channel::Sender<ProgressUpdate>>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<ScanResult, String> {
    let started = std::time::Instant::now();
    progress::send(progress_tx, "collect", 0, 0, "正在遍历目录...");
    let entries = scan::collect_entries(opts, progress_tx, cancel)?;
    progress::send(progress_tx, "collect", entries.len() as u64, entries.len() as u64, "目录遍历完成");

    let mut cache_opt = None;
    if opts.use_cache && !opts.cache_path.is_empty() {
        match cache::HashCache::open(&opts.cache_path) {
            Ok(c) => cache_opt = Some(c),
            Err(e) => eprintln!("[dedup-core] 缓存打开失败，继续无缓存扫描: {e}"),
        }
    }

    let (groups, cache_hits) = hash::find_duplicates(&entries, opts, cache_opt.as_mut(), progress_tx, cancel)?;

    Ok(ScanResult {
        groups,
        scanned_files: entries.len() as u64,
        scanned_bytes: entries.iter().map(|e| e.size).sum(),
        cache_hits,
        elapsed_ms: started.elapsed().as_millis(),
    })
}
