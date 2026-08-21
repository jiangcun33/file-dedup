//! 相似视频查找：FFmpeg 抽帧 → 帧感知哈希序列 → 序列匹配
//!
//! 算法：
//! - 用 ffmpeg.exe 按时长均匀抽帧（≤24 帧），缩放到 64×64 灰度
//! - 每帧用 [`crate::images::phash_gray`] 计算感知哈希，得到视频签名（帧哈希序列）
//! - 两个视频的签名两两比较帧哈希（汉明距离 ≤ 阈值），双向匹配率 ≥ 60% 视为相似
//! - 签名缓存到 SQLite（video_cache），二次扫描免 ffmpeg 解码
//!
//! ffmpeg 查找顺序：用户指定路径 → 应用目录 → PATH；找不到则返回空结果并提示。

use crate::cache::HashCache;
use crate::hash::sort_by_keep;
use crate::images::hamming;
use crate::images::phash_gray;
use crate::models::{DuplicateGroup, FileEntry, GroupKind, ProgressUpdate, ScanOptions};
use crate::progress::{is_cancelled, send};
use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::AtomicBool;

pub const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "m4v", "mpg", "mpeg", "3gp", "m2ts", "rmvb"];

const MAX_FRAMES: u32 = 24;
const FRAME_SIZE: u32 = 64;
const FRAME_BYTES: usize = (FRAME_SIZE * FRAME_SIZE) as usize;
/// 相似判定：帧匹配率 ≥ 此值视为同一/相似视频
const MATCH_RATIO: f64 = 0.6;

pub fn is_video_ext(ext: Option<&str>) -> bool {
    match ext {
        Some(e) => VIDEO_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)),
        None => false,
    }
}

/// 查找 ffmpeg 可执行文件
pub fn find_ffmpeg(explicit: &str) -> Option<String> {
    if !explicit.is_empty() && std::path::Path::new(explicit).is_file() {
        return Some(explicit.to_string());
    }
    // 应用目录（当前 exe 同目录）
    if let Ok(cur) = std::env::current_exe() {
        if let Some(dir) = cur.parent() {
            let p = dir.join("ffmpeg.exe");
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    // PATH
    if let Ok(path) = std::env::var("PATH") {
        for d in std::env::split_paths(&path) {
            let p = d.join("ffmpeg.exe");
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// 解析 ffmpeg -i 输出的时长（秒）
fn parse_duration(stderr: &str) -> Option<f64> {
    let line = stderr.lines().find(|l| l.contains("Duration:"))?;
    let rest = line.split("Duration:").nth(1)?;
    let t = rest.trim_start().split(',').next()?;
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].trim().parse().ok()?;
    let m: f64 = parts[1].trim().parse().ok()?;
    let s: f64 = parts[2].trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// 提取视频的帧哈希签名；失败返回 None
pub fn extract_signature(ffmpeg: &str, path: &str) -> Option<Vec<u64>> {
    // 先探测时长
    let probe = Command::new(ffmpeg).arg("-i").arg(path).output().ok()?;
    let stderr = String::from_utf8_lossy(&probe.stderr);
    let duration = parse_duration(&stderr)?;
    let fps = (duration / MAX_FRAMES as f64).max(0.5);

    // 抽帧输出原始灰度帧
    let vf = format!("fps={fps:.4},scale={FRAME_SIZE}:{FRAME_SIZE},format=gray");
    let args: Vec<String> = vec![
        "-v".into(),
        "error".into(),
        "-i".into(),
        path.into(),
        "-vf".into(),
        vf,
        "-f".into(),
        "rawvideo".into(),
        "-frames:v".into(),
        MAX_FRAMES.to_string(),
        "-".into(),
    ];
    let out = Command::new(ffmpeg).args(&args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let mut hashes = Vec::new();
    for chunk in out.stdout.chunks(FRAME_BYTES) {
        if chunk.len() == FRAME_BYTES {
            hashes.push(phash_gray(chunk, FRAME_SIZE, FRAME_SIZE));
        }
    }
    if hashes.is_empty() {
        None
    } else {
        Some(hashes)
    }
}

/// 两个视频签名的相似度（0.0 - 1.0）：双向帧匹配率的均值
pub fn videos_similar(a: &[u64], b: &[u64], threshold: u32) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let match_a = a
        .iter()
        .filter(|fa| b.iter().any(|fb| hamming(**fa, *fb) <= threshold))
        .count();
    let match_b = b
        .iter()
        .filter(|fb| a.iter().any(|fa| hamming(*fa, **fb) <= threshold))
        .count();
    (match_a as f64 / a.len() as f64 + match_b as f64 / b.len() as f64) / 2.0
}

pub fn sig_to_string(sig: &[u64]) -> String {
    sig.iter().map(|h| format!("{h:016x}")).collect::<Vec<_>>().join(",")
}

pub fn sig_from_string(s: &str) -> Option<Vec<u64>> {
    s.split(',')
        .map(|h| u64::from_str_radix(h, 16).ok())
        .collect::<Option<Vec<u64>>>()
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect() }
    }
    fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.parent[r] != r {
            r = self.parent[r];
        }
        let mut cur = x;
        while self.parent[cur] != cur {
            let next = self.parent[cur];
            self.parent[cur] = r;
            cur = next;
        }
        r
    }
    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

/// 找出相似视频分组（仅报告 ≥2 个文件的组）
pub fn find_similar_videos(
    entries: &[FileEntry],
    opts: &ScanOptions,
    mut cache: Option<&mut HashCache>,
    progress_tx: Option<&Sender<ProgressUpdate>>,
    cancel: Option<&AtomicBool>,
) -> Vec<DuplicateGroup> {
    let videos: Vec<&FileEntry> = entries
        .iter()
        .filter(|e| {
            let ext = std::path::Path::new(&e.path).extension().and_then(|x| x.to_str());
            is_video_ext(ext)
        })
        .collect();
    if videos.len() < 2 {
        return Vec::new();
    }

    let ffmpeg = match find_ffmpeg(&opts.ffmpeg_path) {
        Some(f) => f,
        None => {
            send(progress_tx, "videos", 0, 0, "未找到 ffmpeg，相似视频查找已跳过（请将 ffmpeg.exe 放到应用目录）");
            return Vec::new();
        }
    };
    send(progress_tx, "videos", 0, videos.len() as u64, "正在提取视频帧签名...");

    // 提取每个视频的签名（带缓存）
    let mut sigs: Vec<Option<Vec<u64>>> = Vec::with_capacity(videos.len());
    for (i, e) in videos.iter().enumerate() {
        if is_cancelled(cancel) {
            return Vec::new();
        }
        let cached = cache
            .as_mut()
            .and_then(|c| c.get_video_sig(&e.path, e.size, e.modified).ok().flatten());
        match cached {
            Some(sig) => {
                sigs.push(Some(sig));
                if let Some(c) = cache.as_mut() {
                    c.hits += 1;
                }
            }
            None => {
                let sig = extract_signature(&ffmpeg, &e.path);
                if let Some(s) = &sig {
                    if let Some(c) = cache.as_mut() {
                        let _ = c.put_video_sig(&e.path, e.size, e.modified, s);
                    }
                }
                sigs.push(sig);
            }
        }
        if i % 5 == 0 || i + 1 == videos.len() {
            send(progress_tx, "videos", i as u64 + 1, videos.len() as u64, &format!("视频签名 {}/{}", i + 1, videos.len()));
        }
    }
    if is_cancelled(cancel) {
        return Vec::new();
    }

    // 两两比较
    let threshold = opts.video_threshold.min(64);
    let mut uf = UnionFind::new(videos.len());
    let mut pairs = 0usize;
    for i in 0..videos.len() {
        if is_cancelled(cancel) {
            break;
        }
        let Some(si) = &sigs[i] else { continue };
        for j in (i + 1)..videos.len() {
            let Some(sj) = &sigs[j] else { continue };
            if videos_similar(si, sj, threshold) >= MATCH_RATIO {
                uf.union(i, j);
                pairs += 1;
            }
        }
    }
    send(progress_tx, "videos", videos.len() as u64, videos.len() as u64, &format!("视频匹配完成：{pairs} 对相似"));

    // 聚合分组
    let mut groups_map: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..videos.len() {
        if sigs[i].is_none() {
            continue;
        }
        let root = uf.find(i);
        groups_map.entry(root).or_default().push(i);
    }
    let mut groups = Vec::new();
    for (_root, idxs) in groups_map {
        if idxs.len() < 2 {
            continue;
        }
        let mut files: Vec<FileEntry> = idxs.iter().map(|&i| videos[i].clone()).collect();
        sort_by_keep(&mut files, opts.keep_strategy);
        groups.push(DuplicateGroup::new(files, GroupKind::SimilarVideo));
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_roundtrip() {
        let sig = vec![0x0123456789abcdef, 0xfedcba9876543210];
        assert_eq!(sig_from_string(&sig_to_string(&sig)).unwrap(), sig);
    }

    #[test]
    fn similar_sequences_match() {
        let a = vec![10u64, 20, 30, 40, 50, 60, 70, 80];
        let b = vec![11u64, 21, 31, 41, 51, 61, 71, 81]; // 每帧仅差 1 位
        let r = videos_similar(&a, &b, 4);
        assert!(r >= 0.9, "近邻帧序列应高度相似，实际 {r}");
    }

    #[test]
    fn unrelated_sequences_low() {
        let a = vec![0u64, 0, 0, 0, 0, 0, 0, 0];
        let b = vec![u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX];
        let r = videos_similar(&a, &b, 4);
        assert!(r < 0.3, "无关视频应低相似度，实际 {r}");
    }

    #[test]
    fn duration_parse() {
        assert!((parse_duration("  Duration: 00:01:30.50, start: 0.000000").unwrap() - 90.5).abs() < 0.01);
        assert_eq!(parse_duration("no duration here"), None);
    }
}
