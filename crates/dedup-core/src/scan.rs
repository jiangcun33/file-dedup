//! 目录遍历与文件收集

use crate::models::{FileEntry, ProgressUpdate, ScanOptions};
use crate::progress::{is_cancelled, send};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use walkdir::WalkDir;

fn ext_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
}

fn matches_any(name: &str, list: &[String]) -> bool {
    list.iter().any(|x| x.eq_ignore_ascii_case(name))
}

/// 收集符合过滤条件的文件条目
pub fn collect_entries(
    opts: &ScanOptions,
    progress_tx: Option<&crossbeam_channel::Sender<ProgressUpdate>>,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();
    let mut count: u64 = 0;

    for root in &opts.paths {
        if is_cancelled(cancel) {
            return Ok(entries);
        }
        let root_path = Path::new(root);
        if !root_path.exists() {
            return Err(format!("路径不存在: {root}"));
        }
        if !root_path.is_dir() {
            return Err(format!("不是目录: {root}"));
        }

        let mut walker = WalkDir::new(root_path);
        if let Some(depth) = opts.max_depth {
            walker = walker.max_depth(depth);
        }
        walker = walker.follow_links(opts.follow_symlinks);

        for item in walker.into_iter().filter_map(|e| e.ok()) {
            if is_cancelled(cancel) {
                return Ok(entries);
            }
            count += 1;
            if count % 500 == 0 {
                send(progress_tx, "collect", count, 0, &format!("已扫描 {count} 个条目"));
            }
            if !item.file_type().is_file() {
                continue;
            }
            let path = item.path();
            let path_str = path.to_string_lossy();

            // 排除路径子串（用于排除目录）
            if opts
                .exclude_paths
                .iter()
                .any(|p| !p.is_empty() && path_str.contains(p.as_str()))
            {
                continue;
            }

            let meta = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.is_file() {
                continue;
            }
            let size = meta.len();
            if opts.min_size > 0 && size < opts.min_size {
                continue;
            }
            if opts.max_size > 0 && size > opts.max_size {
                continue;
            }

            // 扩展名过滤
            let ext = ext_of(path);
            if !opts.only_extensions.is_empty() {
                match &ext {
                    Some(e) if matches_any(e, &opts.only_extensions) => {}
                    _ => continue,
                }
            }
            if let Some(e) = &ext {
                if matches_any(e, &opts.exclude_extensions) {
                    continue;
                }
            }

            entries.push(FileEntry::from_meta(path.to_path_buf(), &meta));
        }
    }
    Ok(entries)
}
