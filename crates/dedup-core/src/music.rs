//! 音乐标签去重：读取音频标签（艺术家/标题），按词相似度 + 时长相近度匹配
//!
//! 匹配规则：
//! - 先按时长分桶（±15% 或两者时长为 0），避免全量两两比较
//! - 桶内两两比较 "艺术家 + 标题" 的词相似度（复用 fuzzy 算法）
//! - 相似度 ≥ 阈值（默认 80%）视为同一首歌的不同副本

use crate::fuzzy::filename_similarity;
use crate::hash::sort_by_keep;
use crate::models::{DuplicateGroup, FileEntry, GroupKind, ScanOptions};
use std::collections::HashMap;

pub const MUSIC_EXTS: &[&str] = &["mp3", "flac", "m4a", "aac", "ogg", "opus", "wma", "wav", "ape", "wv"];

pub fn is_music_ext(ext: Option<&str>) -> bool {
    match ext {
        Some(e) => MUSIC_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)),
        None => false,
    }
}

/// 音乐文件标签信息
#[derive(Debug, Clone, Default)]
pub struct TagInfo {
    pub artist: String,
    pub title: String,
    pub album: String,
    /// 时长（秒），未知为 0
    pub duration: u64,
}

impl TagInfo {
    /// 可比较性：至少有一个文本标签（艺术家或标题）
    fn comparable(&self) -> bool {
        !self.artist.trim().is_empty() || !self.title.trim().is_empty()
    }
}

/// 读取音频文件标签；无标签或无法解析返回 None
pub fn read_tags(path: &str) -> Option<TagInfo> {
    use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};
    let tagged = lofty::probe::Probe::open(path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let artist = tag.artist().unwrap_or_default().to_string();
    let title = tag.title().unwrap_or_default().to_string();
    let album = tag.album().unwrap_or_default().to_string();
    let duration = tagged.properties().duration().as_secs();
    Some(TagInfo { artist, title, album, duration })
}

/// 时长是否在同一桶（±15% 或两者未知）
fn duration_in_same_bucket(a: u64, b: u64) -> bool {
    if a == 0 || b == 0 {
        return true;
    }
    let (big, small) = if a > b { (a, b) } else { (b, a) };
    small >= big * 85 / 100
}

/// 两首歌的相似度（0.0 - 1.0）：文本标签词相似度与时长相近度取加权
pub fn music_similarity(a: &TagInfo, b: &TagInfo) -> f64 {
    let text_a = format!("{} {}", a.artist, a.title);
    let text_b = format!("{} {}", b.artist, b.title);
    let text_sim = filename_similarity(&text_a, &text_b, true);
    // 时长相近加分（±10% 视为完全一致）
    let duration_sim = if a.duration == 0 || b.duration == 0 {
        0.0
    } else {
        let (big, small) = if a.duration > b.duration { (a.duration, b.duration) } else { (b.duration, a.duration) };
        if small >= big * 9 / 10 {
            1.0
        } else if small >= big * 8 / 10 {
            0.5
        } else {
            0.0
        }
    };
    // 文本相似度为主（90%），时长为辅（10%）
    text_sim * 0.9 + duration_sim * 0.1
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

/// 找出音乐标签重复的分组（仅报告 ≥2 个文件的组）
pub fn find_music_duplicates(entries: &[FileEntry], opts: &ScanOptions) -> Vec<DuplicateGroup> {
    let threshold = opts.music_threshold.max(1).min(100) as f64 / 100.0;

    // 读取所有音乐文件的标签
    let mut infos: Vec<Option<TagInfo>> = Vec::with_capacity(entries.len());
    let mut valid: Vec<usize> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let ext = std::path::Path::new(&e.path).extension().and_then(|x| x.to_str());
        if !is_music_ext(ext) {
            infos.push(None);
            continue;
        }
        match read_tags(&e.path) {
            Some(t) if t.comparable() => {
                infos.push(Some(t));
                valid.push(i);
            }
            _ => infos.push(None),
        }
    }
    if valid.len() < 2 {
        return Vec::new();
    }

    // 按时长分桶
    let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
    for &i in &valid {
        let dur = infos[i].as_ref().map(|t| t.duration / 10).unwrap_or(0);
        buckets.entry(dur).or_default().push(i);
    }
    // 时长相近的桶合并比较：直接全量两两比较（音乐文件数量通常有限）
    let mut uf = UnionFind::new(entries.len());
    for a in 0..valid.len() {
        for b in (a + 1)..valid.len() {
            let ia = valid[a];
            let ib = valid[b];
            let (ta, tb) = (infos[ia].as_ref().unwrap(), infos[ib].as_ref().unwrap());
            if !duration_in_same_bucket(ta.duration, tb.duration) {
                continue;
            }
            if music_similarity(ta, tb) >= threshold {
                uf.union(ia, ib);
            }
        }
    }

    // 聚合分组
    let mut groups_map: HashMap<usize, Vec<usize>> = HashMap::new();
    for &i in &valid {
        let root = uf.find(i);
        groups_map.entry(root).or_default().push(i);
    }

    let mut groups = Vec::new();
    for (_root, idxs) in groups_map {
        if idxs.len() < 2 {
            continue;
        }
        let mut files: Vec<FileEntry> = idxs.iter().map(|&i| entries[i].clone()).collect();
        sort_by_keep(&mut files, opts.keep_strategy);
        groups.push(DuplicateGroup::new(files, GroupKind::MusicTag));
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_song_different_bitrate() {
        let a = TagInfo { artist: "周杰伦".into(), title: "晴天".into(), album: "叶惠美".into(), duration: 269 };
        let b = TagInfo { artist: "周杰伦".into(), title: "晴天".into(), album: "叶惠美".into(), duration: 269 };
        assert!(music_similarity(&a, &b) > 0.9);
    }

    #[test]
    fn different_songs_low_similarity() {
        let a = TagInfo { artist: "周杰伦".into(), title: "晴天".into(), album: String::new(), duration: 269 };
        let b = TagInfo { artist: "林俊杰".into(), title: "江南".into(), album: String::new(), duration: 300 };
        assert!(music_similarity(&a, &b) < 0.5, "不同歌曲相似度应较低，实际 {}", music_similarity(&a, &b));
    }

    #[test]
    fn duration_bucket_works() {
        assert!(duration_in_same_bucket(269, 300)); // 相差 ~11% → 同一桶
        assert!(duration_in_same_bucket(269, 240)); // 相差 ~11% → 同一桶
        assert!(!duration_in_same_bucket(269, 200)); // 相差 26% → 不同桶
    }
}
