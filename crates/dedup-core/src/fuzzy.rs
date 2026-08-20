//! 文件名模糊匹配（dupeGuru 式词相似度算法）
//!
//! 算法：
//! - 文件名拆词（字母数字串 + 单个汉字各算一词）
//! - 相似度 = 相同词数 × 2 / 两文件总词数（完全相同词优先，编辑距离 ≤1 的相似词也计入）
//! - 默认仅在同一目录内比较，且要求文件大小相差不超过 4 倍（剪枝，避免 O(n²) 全量比较）

use crate::hash::sort_by_keep;
use crate::models::{DuplicateGroup, FileEntry, GroupKind, ScanOptions};
use std::collections::HashMap;

/// 将文件名拆成词序列：字母数字连续串为一个词，每个汉字单独一个词
pub fn words(s: &str) -> Vec<String> {
    let lower = s.to_lowercase();
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() {
            cur.push(ch);
        } else if is_cjk(ch) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(ch.to_string());
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn is_cjk(ch: char) -> bool {
    let c = ch as u32;
    (0x4E00..=0x9FFF).contains(&c)
        || (0x3400..=0x4DBF).contains(&c)
        || (0xF900..=0xFAFF).contains(&c)
}

/// 文件名相似度（0.0 - 1.0），dupeGuru 词相似度算法
/// `similar_words`：编辑距离 ≤1 的相似词是否计入匹配
pub fn filename_similarity(a: &str, b: &str, similar_words: bool) -> f64 {
    let wa = words(a);
    let wb = words(b);
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let mut used_b = vec![false; wb.len()];
    let mut common = 0usize;
    for x in &wa {
        // 优先精确匹配，其次（可选）编辑距离 ≤1 的相似词
        let exact = wb
            .iter()
            .enumerate()
            .find(|(i, y)| !used_b[*i] && **y == *x)
            .map(|(i, _)| i);
        let idx = match exact {
            Some(i) => Some(i),
            None if similar_words => {
                // 相似词（编辑距离 ≤1）仅对长度 ≥2 的词生效：
                // 单汉字任意两个都距离 1，若允许会破坏中文文件名的匹配精度
                if x.chars().count() < 2 {
                    None
                } else {
                    wb.iter()
                        .enumerate()
                        .find(|(i, y)| !used_b[*i] && y.chars().count() >= 2 && strsim::levenshtein(x, y) <= 1)
                        .map(|(i, _)| i)
                }
            }
            None => None,
        };
        if let Some(i) = idx {
            used_b[i] = true;
            common += 1;
        }
    }
    common as f64 * 2.0 / (wa.len() as f64 + wb.len() as f64)
}

fn parent_dir(path: &str) -> &str {
    let idx = path.rfind(['/', '\\']).unwrap_or(0);
    &path[..idx]
}

/// 取文件名（不含路径）
fn base_name(path: &str) -> &str {
    let idx = path.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0);
    &path[idx..]
}

fn size_ratio_ok(a: u64, b: u64) -> bool {
    if a == 0 || b == 0 {
        return a == b;
    }
    let (big, small) = if a > b { (a, b) } else { (b, a) };
    big <= small * 4
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

/// 找出文件名模糊匹配的分组（仅报告 ≥2 个文件的组）
pub fn find_fuzzy_name_duplicates(entries: &[FileEntry], opts: &ScanOptions) -> Vec<DuplicateGroup> {
    let threshold = opts.fuzzy_threshold.max(1).min(100) as f64 / 100.0;

    // 按父目录分桶（或全局一个桶）
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        let key = if opts.fuzzy_same_dir_only {
            parent_dir(&e.path).to_string()
        } else {
            String::new()
        };
        buckets.entry(key).or_default().push(i);
    }

    let mut uf = UnionFind::new(entries.len());
    for (_dir, idxs) in &buckets {
        if idxs.len() < 2 {
            continue;
        }
        if idxs.len() > 3000 {
            // 超大目录跳过模糊匹配，避免 O(n²)
            continue;
        }
        for a in 0..idxs.len() {
            for b in (a + 1)..idxs.len() {
                let ea = &entries[idxs[a]];
                let eb = &entries[idxs[b]];
                if !size_ratio_ok(ea.size, eb.size) {
                    continue;
                }
                if filename_similarity(base_name(&ea.path), base_name(&eb.path), true) >= threshold {
                    uf.union(idxs[a], idxs[b]);
                }
            }
        }
    }

    // 按并查集根聚合
    let mut groups_map: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..entries.len() {
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
        groups.push(DuplicateGroup::new(files, GroupKind::FuzzyName));
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_dupeguru_example() {
        // dupeGuru 文档示例（不启用相似词）："a b c d" 与 "c d e" → 57%
        let sim = filename_similarity("a b c d", "c d e", false);
        assert!((sim - 4.0 / 7.0).abs() < 0.02, "期望约 0.571，实际 {sim}");
    }

    #[test]
    fn similarity_identical_names() {
        assert_eq!(filename_similarity("report.txt", "report.txt", true), 1.0);
    }

    #[test]
    fn similarity_copy_variant() {
        // "报告" 与 "报告 副本"
        let sim = filename_similarity("报告.txt", "报告 副本.txt", true);
        assert!(sim > 0.5, "期望较高相似度，实际 {sim}");
    }

    #[test]
    fn words_split_underscore() {
        assert_eq!(words("report_final_v2.txt"), vec!["report", "final", "v2", "txt"]);
    }

    #[test]
    fn words_chinese() {
        assert_eq!(words("文档A.txt"), vec!["文", "档", "a", "txt"]);
    }
}
