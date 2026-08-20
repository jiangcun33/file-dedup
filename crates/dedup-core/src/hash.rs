//! 三级漏斗哈希与重复分组
//!
//! 阶段1：按文件大小分组（唯一大小直接淘汰）
//! 阶段2：部分哈希（≥64KB 读头尾各 4KB；小文件直接整读，顺带算全哈希）
//! 阶段3：全文件哈希（blake3 流式）确认字节级一致
//!
//! 哈希缓存：以 (path, size, mtime) 为键，缓存部分哈希与全哈希，避免重复计算。

use crate::cache::HashCache;
use crate::models::{DuplicateGroup, FileEntry, KeepStrategy, ProgressUpdate, ScanOptions};
use crate::progress::{is_cancelled, send};
use crossbeam_channel::Sender;
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::AtomicBool;

const PARTIAL_THRESHOLD: u64 = 64 * 1024;
const HEAD_TAIL_SIZE: u64 = 4096;
const READ_BUF_SIZE: usize = 1024 * 1024;

fn read_head(path: &str, n: u64) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; n as usize];
    let read = f.read(&mut buf)?;
    buf.truncate(read);
    Ok(buf)
}

fn read_tail(path: &str, n: u64, size: u64) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    let start = size.saturating_sub(n);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; n as usize];
    let read = f.read(&mut buf)?;
    buf.truncate(read);
    Ok(buf)
}

fn read_whole(path: &str) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// 流式计算 blake3 全文件哈希
fn blake3_full(path: &str) -> std::io::Result<[u8; 32]> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; READ_BUF_SIZE];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// 计算部分哈希；小文件顺带返回全哈希
fn compute_partial(entry: &FileEntry) -> std::io::Result<(u128, Option<[u8; 32]>)> {
    use xxhash_rust::xxh3::xxh3_128;
    if entry.size < PARTIAL_THRESHOLD {
        let bytes = read_whole(&entry.path)?;
        let partial = xxh3_128(&bytes);
        let full = blake3::hash(&bytes);
        Ok((partial, Some(*full.as_bytes())))
    } else {
        let head = read_head(&entry.path, HEAD_TAIL_SIZE)?;
        let tail = read_tail(&entry.path, HEAD_TAIL_SIZE, entry.size)?;
        let mut buf = Vec::with_capacity(head.len() + tail.len() + 8);
        buf.extend_from_slice(&entry.size.to_le_bytes());
        buf.extend_from_slice(&head);
        buf.extend_from_slice(&tail);
        Ok((xxh3_128(&buf), None))
    }
}

/// 对组内文件按保留策略排序，files[0] 即参考文件
fn sort_by_keep(files: &mut Vec<FileEntry>, strategy: KeepStrategy) {
    match strategy {
        KeepStrategy::KeepNewest => files.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path))),
        KeepStrategy::KeepOldest => files.sort_by(|a, b| a.modified.cmp(&b.modified).then(a.path.cmp(&b.path))),
        KeepStrategy::KeepLargest => files.sort_by(|a, b| b.size.cmp(&a.size).then(a.path.cmp(&b.path))),
        KeepStrategy::KeepFirst => {}
    }
}

/// 主入口：从条目列表找出重复分组
pub fn find_duplicates(
    entries: &[FileEntry],
    opts: &ScanOptions,
    mut cache: Option<&mut HashCache>,
    progress_tx: Option<&Sender<ProgressUpdate>>,
    cancel: Option<&AtomicBool>,
) -> Result<(Vec<DuplicateGroup>, u64), String> {
    let total = entries.len() as u64;

    // ---- 阶段1：按大小分组 ----
    let mut by_size: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    for e in entries {
        by_size.entry(e.size).or_default().push(e.clone());
    }
    let size_groups: Vec<Vec<FileEntry>> = by_size
        .into_values()
        .filter(|v| v.len() >= 2)
        .collect();
    send(progress_tx, "hash", 1, 4, &format!("大小分组完成：{} 组候选", size_groups.len()));

    // ---- 阶段2：部分哈希 ----
    let mut by_partial: HashMap<(u64, u128), Vec<FileEntry>> = HashMap::new();
    let mut processed: u64 = 0;
    for group in &size_groups {
        if is_cancelled(cancel) {
            return Ok((Vec::new(), cache.as_ref().map(|c| c.hits).unwrap_or(0)));
        }
        // 缓存命中查询（顺序）
        let mut to_compute: Vec<FileEntry> = Vec::new();
        let mut partials: Vec<(FileEntry, u128)> = Vec::with_capacity(group.len());
        for f in group {
            let cached = cache
                .as_mut()
                .and_then(|c| c.get(&f.path, f.size, f.modified).ok().flatten());
            match cached {
                Some(c) if c.partial.is_some() => {
                    if let Some(cache) = cache.as_mut() {
                        cache.hits += 1;
                    }
                    partials.push((f.clone(), c.partial.unwrap()));
                }
                _ => to_compute.push(f.clone()),
            }
        }
        // 并行计算缺失的部分哈希
        let computed: Vec<std::io::Result<(u128, Option<[u8; 32]>)>> =
            to_compute.par_iter().map(compute_partial).collect();
        for (f, r) in to_compute.into_iter().zip(computed) {
            match r {
                Ok((p, full)) => {
                    if let Some(c) = cache.as_mut() {
                        let _ = c.put(&f.path, f.size, f.modified, Some(p), full);
                    }
                    partials.push((f, p));
                }
                Err(e) => eprintln!("[dedup-core] 哈希失败 {}: {e}", f.path),
            }
        }
        for (f, p) in partials {
            let key = (f.size, p);
            by_partial.entry(key).or_default().push(f);
        }
        processed += group.len() as u64;
        if processed % 1000 == 0 || processed == total {
            send(progress_tx, "hash", 2, 4, &format!("部分哈希 {processed}/{total}"));
        }
    }

    // ---- 阶段3：全文件哈希 ----
    let partial_groups: Vec<Vec<FileEntry>> = by_partial
        .into_values()
        .filter(|v| v.len() >= 2)
        .collect();
    send(progress_tx, "hash", 3, 4, &format!("部分哈希匹配完成：{} 组候选", partial_groups.len()));

    let mut final_map: HashMap<(u64, [u8; 32]), Vec<FileEntry>> = HashMap::new();
    let mut processed3: u64 = 0;
    let total3 = partial_groups.iter().map(|g| g.len() as u64).sum::<u64>();
    for group in &partial_groups {
        if is_cancelled(cancel) {
            return Ok((Vec::new(), cache.as_ref().map(|c| c.hits).unwrap_or(0)));
        }
        let mut to_compute: Vec<FileEntry> = Vec::new();
        let mut fulls: Vec<(FileEntry, [u8; 32])> = Vec::with_capacity(group.len());
        for f in group {
            let cached_full = cache
                .as_mut()
                .and_then(|c| c.get(&f.path, f.size, f.modified).ok().flatten())
                .and_then(|c| c.full);
            match cached_full {
                Some(full) => {
                    if let Some(cache) = cache.as_mut() {
                        cache.hits += 1;
                    }
                    fulls.push((f.clone(), full));
                }
                None => to_compute.push(f.clone()),
            }
        }
        let computed: Vec<std::io::Result<[u8; 32]>> = to_compute.par_iter().map(|f| blake3_full(&f.path)).collect();
        for (f, r) in to_compute.into_iter().zip(computed) {
            match r {
                Ok(full) => {
                    if let Some(c) = cache.as_mut() {
                        let _ = c.put(&f.path, f.size, f.modified, None, Some(full));
                    }
                    fulls.push((f, full));
                }
                Err(e) => eprintln!("[dedup-core] 全哈希失败 {}: {e}", f.path),
            }
        }
        // 同组文件共享同一个部分哈希，按 (size, full) 即可正确分组
        for (f, full) in fulls {
            final_map.entry((f.size, full)).or_default().push(f);
        }
        processed3 += group.len() as u64;
        if processed3 % 1000 == 0 || processed3 == total3 {
            send(progress_tx, "hash", 3, 4, &format!("全文件哈希 {processed3}/{total3}"));
        }
    }

    // ---- 组装最终分组 ----
    let mut groups = Vec::new();
    for mut files in final_map.into_values() {
        if files.len() < 2 {
            continue;
        }
        sort_by_keep(&mut files, opts.keep_strategy);
        groups.push(DuplicateGroup::new(files));
    }
    groups.sort_by(|a, b| b.reclaimable.cmp(&a.reclaimable));
    send(progress_tx, "hash", 4, 4, &format!("完成：找到 {} 组重复", groups.len()));

    let hits = cache.as_ref().map(|c| c.hits).unwrap_or(0);
    Ok((groups, hits))
}
