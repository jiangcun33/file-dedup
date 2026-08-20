//! 相似图片查找：pHash（DCT 感知哈希）+ BK 树近邻匹配
//!
//! 算法：
//! - 图片解码 → 32×32 灰度 → 二维 DCT → 取 8×8 低频块 → 中位数阈值 → 64 位哈希
//! - 汉明距离 ≤ 阈值视为相似（默认 10/64）
//! - 哈希值缓存到 SQLite，二次扫描跳过解码
//! - BK 树索引全部哈希，按半径查询近邻，再以并查集聚类成组

use crate::cache::HashCache;
use crate::hash::sort_by_keep;
use crate::models::{DuplicateGroup, FileEntry, GroupKind, ProgressUpdate, ScanOptions};
use crate::progress::{is_cancelled, send};
use crossbeam_channel::Sender;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "bmp", "webp", "ico"];

pub fn is_image_ext(ext: Option<&str>) -> bool {
    match ext {
        Some(e) => IMAGE_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)),
        None => false,
    }
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 一维 DCT-II（正交归一化系数在比较中可抵消，仅需相对大小）
fn dct1d(input: &[f64], out: &mut [f64]) {
    let n = input.len();
    let pi = std::f64::consts::PI;
    for k in 0..n {
        let mut sum = 0.0;
        for x in 0..n {
            sum += input[x] * ((2.0 * x as f64 + 1.0) * k as f64 * pi / (2.0 * n as f64)).cos();
        }
        let ck = if k == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
        out[k] = ck * sum;
    }
}

/// 计算图片的 64 位感知哈希；解码失败返回 None
///
/// 算法：32×32 灰度 → 二维 DCT → 取 8×8 低频块 → 去掉 DC（亮度）项后，
/// 以其余 63 个 AC 系数的均值为阈值转成位签名。对亮度偏移与微小扰动鲁棒，
/// 且能区分平滑但不同的图像。
pub fn phash(path: &str) -> Option<u64> {
    let img = image::open(path).ok()?;
    let small = img
        .resize_exact(32, 32, image::imageops::FilterType::Triangle)
        .to_luma8();
    let mut m = [[0f64; 32]; 32];
    for y in 0..32 {
        for x in 0..32 {
            m[y][x] = small.get_pixel(x as u32, y as u32)[0] as f64;
        }
    }
    // 行方向 DCT
    let mut tmp = [[0f64; 32]; 32];
    let mut row_out = [0f64; 32];
    for y in 0..32 {
        dct1d(&m[y], &mut row_out);
        tmp[y] = row_out;
    }
    // 列方向 DCT
    let mut dct = [[0f64; 32]; 32];
    let mut col = [0f64; 32];
    let mut col_out = [0f64; 32];
    for x in 0..32 {
        for y in 0..32 {
            col[y] = tmp[y][x];
        }
        dct1d(&col, &mut col_out);
        for y in 0..32 {
            dct[y][x] = col_out[y];
        }
    }
    // 取 8×8 低频块，去掉 DC 后按 AC 均值转位
    let mut block = [0f64; 64];
    for y in 0..8 {
        for x in 0..8 {
            block[y * 8 + x] = dct[y][x];
        }
    }
    let ac: &[f64] = &block[1..];
    let mean = ac.iter().sum::<f64>() / ac.len() as f64;
    let mut hash = 0u64;
    for (i, v) in ac.iter().enumerate() {
        if *v > mean {
            hash |= 1u64 << i;
        }
    }
    Some(hash)
}

/// 简单 BK 树：节点存哈希，边按汉明距离
struct BKTree {
    nodes: Vec<u64>,
    children: Vec<HashMap<u32, usize>>,
}

impl BKTree {
    fn new() -> Self {
        Self { nodes: Vec::new(), children: Vec::new() }
    }
    fn insert(&mut self, hash: u64) {
        if self.nodes.is_empty() {
            self.nodes.push(hash);
            self.children.push(HashMap::new());
            return;
        }
        let mut idx = 0;
        loop {
            let d = hamming(self.nodes[idx], hash);
            if d == 0 {
                return; // 已有完全相同哈希（精确重复，由精确去重处理）
            }
            if let Some(&next) = self.children[idx].get(&d) {
                idx = next;
            } else {
                let new_idx = self.nodes.len();
                self.nodes.push(hash);
                self.children.push(HashMap::new());
                self.children[idx].insert(d, new_idx);
                return;
            }
        }
    }
    /// 返回与 hash 汉明距离 ≤ radius 的所有节点索引
    fn query(&self, hash: u64, radius: u32) -> Vec<usize> {
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return out;
        }
        let mut stack = vec![0usize];
        while let Some(idx) = stack.pop() {
            let d = hamming(self.nodes[idx], hash);
            if d <= radius {
                out.push(idx);
            }
            let lo = d.saturating_sub(radius);
            let hi = d + radius;
            for (dist, child) in &self.children[idx] {
                if *dist >= lo && *dist <= hi {
                    stack.push(*child);
                }
            }
        }
        out
    }

    fn node_hash(&self, idx: usize) -> u64 {
        self.nodes[idx]
    }
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

/// 找出相似图片分组（仅报告 ≥2 个文件的组）
pub fn find_similar_images(
    entries: &[FileEntry],
    opts: &ScanOptions,
    mut cache: Option<&mut HashCache>,
    progress_tx: Option<&Sender<ProgressUpdate>>,
    cancel: Option<&AtomicBool>,
) -> Vec<DuplicateGroup> {
    let images: Vec<&FileEntry> = entries
        .iter()
        .filter(|e| {
            let ext = std::path::Path::new(&e.path)
                .extension()
                .and_then(|x| x.to_str());
            is_image_ext(ext)
        })
        .collect();
    if images.len() < 2 {
        return Vec::new();
    }
    send(progress_tx, "images", 0, images.len() as u64, "正在计算图片感知哈希...");

    let threshold = opts.image_threshold.min(64);

    // 计算哈希（并行；未命中的才解码）
    let mut to_compute: Vec<(&FileEntry, usize)> = Vec::new();
    let mut hashes: Vec<Option<u64>> = vec![None; images.len()];
    for (i, e) in images.iter().enumerate() {
        let cached = cache
            .as_mut()
            .and_then(|c| c.get_phash(&e.path, e.size, e.modified).ok().flatten());
        match cached {
            Some(h) => {
                hashes[i] = Some(h);
                if let Some(c) = cache.as_mut() {
                    c.hits += 1;
                }
            }
            None => to_compute.push((e, i)),
        }
    }
    let computed: Vec<Option<u64>> = to_compute
        .par_iter()
        .map(|(e, _)| phash(&e.path))
        .collect();
    for ((e, i), h) in to_compute.into_iter().zip(computed) {
        hashes[i] = h;
        if let Some(h) = h {
            if let Some(c) = cache.as_mut() {
                let _ = c.put_phash(&e.path, e.size, e.modified, h);
            }
        }
    }
    if is_cancelled(cancel) {
        return Vec::new();
    }

    // 建立 BK 树（树节点=唯一哈希）并维护 哈希→图片索引 映射（处理完全相同的哈希）
    let mut tree = BKTree::new();
    let mut hash_indices: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, h) in hashes.iter().enumerate() {
        if let Some(h) = h {
            tree.insert(*h);
            hash_indices.entry(*h).or_default().push(i);
        }
    }
    let mut uf = UnionFind::new(images.len());
    let mut edge_count = 0usize;
    for i in 0..hashes.len() {
        if is_cancelled(cancel) {
            break;
        }
        if let Some(h) = hashes[i] {
            for &node_idx in tree.query(h, threshold).iter() {
                let nh = tree.node_hash(node_idx);
                if let Some(idxs) = hash_indices.get(&nh) {
                    for &j in idxs {
                        if j > i {
                            uf.union(i, j);
                            edge_count += 1;
                        }
                    }
                }
            }
        }
        if i % 200 == 0 {
            send(progress_tx, "images", i as u64, images.len() as u64, &format!("图片比对 {}/{}", i, images.len()));
        }
    }
    send(progress_tx, "images", images.len() as u64, images.len() as u64, &format!("图片匹配完成：{edge_count} 对"));

    // 聚合分组
    let mut groups_map: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..images.len() {
        if hashes[i].is_none() {
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
        let mut files: Vec<FileEntry> = idxs.iter().map(|&i| images[i].clone()).collect();
        sort_by_keep(&mut files, opts.keep_strategy);
        groups.push(DuplicateGroup::new(files, GroupKind::SimilarImage));
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hamming_works() {
        assert_eq!(hamming(0b1010, 0b1111), 2);
        assert_eq!(hamming(0, 0), 0);
    }

    #[test]
    fn bk_tree_query() {
        let mut tree = BKTree::new();
        let h1 = 0b0000_0000u64;
        let h2 = 0b0000_0011u64; // 距 h1 = 2
        let h3 = 0b1111_0000u64; // 距 h1 = 4
        tree.insert(h1);
        tree.insert(h2);
        tree.insert(h3);
        let near = tree.query(h1, 2);
        assert!(near.len() >= 2, "半径2内应至少命中 h1、h2");
        let near4 = tree.query(h1, 4);
        assert_eq!(near4.len(), 3);
    }

    #[test]
    fn phash_same_image_different_size() {
        // 生成一张 100x100 的渐变色 PNG（无溢出），另一张同内容 200x200
        let p1 = make_gradient_png(100, 100, 1);
        let p2 = make_gradient_png(200, 200, 1);
        let h1 = phash(&p1).expect("图片1应可解码");
        let h2 = phash(&p2).expect("图片2应可解码");
        let d = hamming(h1, h2);
        assert!(d <= 6, "同内容不同尺寸的图片哈希应接近，距离 {d}");
        let _ = std::fs::remove_file(&p1);
        let _ = std::fs::remove_file(&p2);
    }

    fn make_gradient_png(w: u32, h: u32, seed: u32) -> String {
        use image::{ImageBuffer, Rgb};
        let mut img = ImageBuffer::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 255 / w) + seed) as u8;
                // 无溢出的平滑渐变
                img.put_pixel(x, y, Rgb([v, v, 255 - v]));
            }
        }
        let path = format!(
            "{}\\phash_test_{}_{}_{}.png",
            std::env::temp_dir().to_string_lossy(),
            std::process::id(),
            seed,
            w
        );
        img.save(&path).expect("保存测试图片失败");
        path
    }
}
