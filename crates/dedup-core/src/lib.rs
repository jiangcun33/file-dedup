//! 文件去重核心引擎（无 UI 依赖）
//!
//! 模块划分：
//! - [`models`]  数据结构定义
//! - [`scan`]    目录遍历与文件收集
//! - [`hash`]    三级漏斗精确去重
//! - [`fuzzy`]   文件名模糊匹配
//! - [`images`]  相似图片查找（感知哈希）
//! - [`cache`]   SQLite 哈希缓存
//! - [`action`]  文件操作（回收站/删除/硬链接/移动/复制）
//! - [`progress`] 进度与取消

pub mod action;
pub mod cache;
pub mod fuzzy;
pub mod hash;
pub mod images;
pub mod models;
pub mod progress;
pub mod scan;

pub use models::{
    ActionItem, ActionKind, ActionRequest, ActionResult, BatchActionRequest, DuplicateGroup, FileEntry, GroupKind,
    KeepStrategy, ProgressUpdate, ScanOptions, ScanResult,
};

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

    // ---- 1. 精确内容去重（始终执行） ----
    let (mut groups, cache_hits) = hash::find_duplicates(&entries, opts, cache_opt.as_mut(), progress_tx, cancel)?;

    // ---- 2/3. 模糊匹配与相似图片（可选） ----
    // 已在精确分组中的文件不再参与，避免重复报告
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for g in &groups {
        for f in &g.files {
            used.insert(f.path.clone());
        }
    }
    let remaining: Vec<FileEntry> = entries
        .iter()
        .filter(|e| !used.contains(&e.path))
        .cloned()
        .collect();

    if opts.fuzzy_filename {
        let fg = fuzzy::find_fuzzy_name_duplicates(&remaining, opts);
        progress::send(
            progress_tx,
            "fuzzy",
            fg.len() as u64,
            fg.len() as u64,
            &format!("文件名模糊匹配完成：{} 组", fg.len()),
        );
        groups.extend(fg);
    }

    if opts.similar_images {
        let ig = images::find_similar_images(&remaining, opts, cache_opt.as_mut(), progress_tx, cancel);
        groups.extend(ig);
    }

    groups.sort_by(|a, b| b.reclaimable.cmp(&a.reclaimable));

    Ok(ScanResult {
        groups,
        scanned_files: entries.len() as u64,
        scanned_bytes: entries.iter().map(|e| e.size).sum(),
        cache_hits,
        elapsed_ms: started.elapsed().as_millis(),
    })
}
