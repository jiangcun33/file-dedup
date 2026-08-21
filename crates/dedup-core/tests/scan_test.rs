//! 核心引擎集成测试：扫描 → 分组 → 缓存 → 操作

use dedup_core::models::{ActionItem, ActionKind, ActionRequest, BatchActionRequest, FileEntry, KeepStrategy, ScanOptions};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dedup-core-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &PathBuf, rel: &str, content: &[u8]) -> PathBuf {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, content).unwrap();
    p
}

fn base_options(paths: Vec<String>) -> ScanOptions {
    ScanOptions {
        paths,
        recursive: true,
        min_size: 0,
        max_size: 0,
        only_extensions: vec![],
        exclude_extensions: vec![],
        exclude_paths: vec![],
        follow_symlinks: false,
        max_depth: None,
        keep_strategy: KeepStrategy::KeepOldest,
        use_cache: false,
        cache_path: String::new(),
        ..Default::default()
    }
}

#[test]
fn finds_exact_duplicates() {
    let dir = test_dir("find");
    let content_a = b"hello duplicate world, this is a test payload";
    let content_b = b"a completely different file body";
    write(&dir, "a.txt", content_a);
    write(&dir, "b.txt", content_a); // 与 a 相同
    write(&dir, "sub/c.txt", content_b);
    write(&dir, "sub/d.txt", content_a); // 子目录中的副本
    write(&dir, "e.bin", b"unique");

    let cancel = AtomicBool::new(false);
    let opts = base_options(vec![dir.to_string_lossy().into_owned()]);
    let result = dedup_core::run_scan(&opts, None, Some(&cancel)).unwrap();

    assert_eq!(result.groups.len(), 1, "应找到 1 组重复，实际 {}", result.groups.len());
    let g = &result.groups[0];
    assert_eq!(g.files.len(), 3, "组内应有 3 个文件");
    assert_eq!(g.reclaimable, 2 * content_a.len() as u64);
    assert_eq!(result.scanned_files, 5);
}

#[test]
fn cache_speeds_up_second_scan() {
    let dir = test_dir("cache");
    // 使用 ≥64KB 的文件：阶段2只缓存部分哈希，阶段3的全哈希必须在首次扫描时真算
    let content = vec![b'x'; 70 * 1024];
    write(&dir, "a.bin", &content);
    write(&dir, "b.bin", &content);
    let cache_path = dir.join("cache.db");
    let cancel = AtomicBool::new(false);
    let mut opts = base_options(vec![dir.to_string_lossy().into_owned()]);
    opts.use_cache = true;
    opts.cache_path = cache_path.to_string_lossy().into_owned();

    let r1 = dedup_core::run_scan(&opts, None, Some(&cancel)).unwrap();
    assert_eq!(r1.groups.len(), 1);
    assert_eq!(r1.cache_hits, 0, "首次扫描不应有缓存命中，实际 {}", r1.cache_hits);

    // 第一次扫描已写入缓存；第二次应全部命中（每文件：阶段2部分哈希 + 阶段3全哈希 = 2 次）
    let r2 = dedup_core::run_scan(&opts, None, Some(&cancel)).unwrap();
    assert_eq!(r2.groups.len(), 1);
    assert_eq!(r2.cache_hits, 4, "二次扫描应全部命中，实际 {}", r2.cache_hits);
}

fn entry_of(p: &PathBuf) -> FileEntry {
    let meta = std::fs::metadata(p).unwrap();
    FileEntry::from_meta(p.clone(), &meta)
}

#[test]
fn hardlink_replace_keeps_content() {
    let dir = test_dir("hardlink");
    let content = b"hardlink payload";
    let a = write(&dir, "a.txt", content);
    let b = write(&dir, "b.txt", content);
    let ref_path = a.to_string_lossy().into_owned();

    let req = ActionRequest {
        kind: ActionKind::Hardlink,
        items: vec![entry_of(&b)],
        reference: ref_path.clone(),
        dest_dir: None,
    };
    let results = dedup_core::action::apply_action(&req);
    assert!(results[0].ok, "硬链接替换失败: {}", results[0].message);
    // b.txt 现在与 a.txt 是同一 inode：内容一致，且改写 a 会反映到 b
    assert_eq!(std::fs::read(&b).unwrap(), content);
    std::fs::write(&a, b"hardlink payload UPDATED").unwrap();
    assert_eq!(
        std::fs::read(&b).unwrap(),
        b"hardlink payload UPDATED",
        "a 与 b 应为同一硬链接（共享数据）"
    );
}

#[test]
fn move_and_copy_work() {
    let dir = test_dir("movecopy");
    let content = b"move copy payload";
    let a = write(&dir, "src/a.txt", content);
    let b = write(&dir, "src/b.txt", content);
    let dest = dir.join("dest");
    std::fs::create_dir_all(&dest).unwrap();
    let dest_str = dest.to_string_lossy().into_owned();

    // 移动
    let req_move = ActionRequest {
        kind: ActionKind::Move,
        items: vec![entry_of(&b)],
        reference: String::new(),
        dest_dir: Some(dest_str.clone()),
    };
    let r = dedup_core::action::apply_action(&req_move);
    assert!(r[0].ok, "移动失败: {}", r[0].message);
    assert!(dest.join("b.txt").exists());
    assert!(!b.exists());

    // 复制
    let req_copy = ActionRequest {
        kind: ActionKind::Copy,
        items: vec![entry_of(&a)],
        reference: String::new(),
        dest_dir: Some(dest_str),
    };
    let r = dedup_core::action::apply_action(&req_copy);
    assert!(r[0].ok, "复制失败: {}", r[0].message);
    assert!(dest.join("a.txt").exists());
    assert!(a.exists());
}

#[test]
fn permanent_delete_works() {
    let dir = test_dir("delete");
    let content = b"delete payload";
    let a = write(&dir, "a.txt", content);
    let req = ActionRequest {
        kind: ActionKind::Delete,
        items: vec![entry_of(&a)],
        reference: String::new(),
        dest_dir: None,
    };
    let r = dedup_core::action::apply_action(&req);
    assert!(r[0].ok, "删除失败: {}", r[0].message);
    assert!(!a.exists());
}

#[test]
fn unchanged_check_blocks_modified_files() {
    let dir = test_dir("tamper");
    let content = b"original payload";
    let a = write(&dir, "a.txt", content);
    let entry = entry_of(&a);
    // 篡改：修改文件内容（不同长度）后使用旧的元数据
    std::fs::write(&a, b"tampered content much longer than before").unwrap();
    let req = ActionRequest {
        kind: ActionKind::Delete,
        items: vec![entry.clone()],
        reference: String::new(),
        dest_dir: None,
    };
    let r = dedup_core::action::apply_action(&req);
    assert!(!r[0].ok, "文件大小已变化，应拒绝操作，实际: {}", r[0].message);
    assert!(a.exists(), "文件不应被删除");
    // 用最新元数据则允许
    let entry2 = entry_of(&a);
    let req2 = ActionRequest {
        kind: ActionKind::Delete,
        items: vec![entry2],
        reference: String::new(),
        dest_dir: None,
    };
    let r2 = dedup_core::action::apply_action(&req2);
    assert!(r2[0].ok, "最新元数据应允许删除: {}", r2[0].message);
}

#[test]
fn batch_action_across_groups() {
    let dir = test_dir("batch");
    let content_a = b"group a payload data";
    let content_b = b"group b payload data";
    // 组1：a1/a2 相同；组2：b1/b2 相同
    let a1 = write(&dir, "a1.txt", content_a);
    let a2 = write(&dir, "a2.txt", content_a);
    let b1 = write(&dir, "b1.txt", content_b);
    let b2 = write(&dir, "b2.txt", content_b);

    // 跨组批量硬链接替换：a2→a1, b2→b1
    let req = BatchActionRequest {
        kind: ActionKind::Hardlink,
        items: vec![
            ActionItem { file: entry_of(&a2), reference: a1.to_string_lossy().into_owned() },
            ActionItem { file: entry_of(&b2), reference: b1.to_string_lossy().into_owned() },
        ],
        dest_dir: None,
    };
    let results = dedup_core::action::apply_batch_action(&req);
    assert_eq!(results.len(), 2);
    for r in &results {
        assert!(r.ok, "批量硬链接失败: {}", r.message);
    }
    // 改写参考后副本同步（证明硬链接生效）
    std::fs::write(&a1, b"group a payload data UPDATED").unwrap();
    std::fs::write(&b1, b"group b payload data UPDATED").unwrap();
    assert_eq!(std::fs::read(&a2).unwrap(), b"group a payload data UPDATED");
    assert_eq!(std::fs::read(&b2).unwrap(), b"group b payload data UPDATED");
}

#[test]
fn fuzzy_filename_matching() {
    let dir = test_dir("fuzzy");
    write(&dir, "report.txt", b"content alpha");
    write(&dir, "report copy.txt", b"content beta different");
    write(&dir, "report final.txt", b"content gamma totally different");
    write(&dir, "unrelated.txt", b"something else entirely");

    let cancel = AtomicBool::new(false);
    let mut opts = base_options(vec![dir.to_string_lossy().into_owned()]);
    opts.fuzzy_filename = true;
    opts.fuzzy_threshold = 60;
    opts.fuzzy_same_dir_only = true;

    let result = dedup_core::run_scan(&opts, None, Some(&cancel)).unwrap();
    let fuzzy: Vec<_> = result.groups.iter().filter(|g| g.kind == dedup_core::GroupKind::FuzzyName).collect();
    assert_eq!(fuzzy.len(), 1, "应找到 1 组文件名模糊匹配，实际 {}", fuzzy.len());
    assert_eq!(fuzzy[0].files.len(), 3, "report* 三兄弟应同组");
    // unrelated 不应混入
    for f in &fuzzy[0].files {
        assert!(!f.path.ends_with("unrelated.txt"), "无关文件不应入组");
    }
}

#[test]
fn fuzzy_similarity_threshold_respects() {
    assert!((dedup_core::fuzzy::filename_similarity("a b c d", "c d e", false) - 4.0 / 7.0).abs() < 0.02);
    assert_eq!(dedup_core::fuzzy::filename_similarity("report.txt", "report.txt", true), 1.0);
}

fn gradient_png(w: u32, h: u32, seed: u32) -> PathBuf {
    use image::{ImageBuffer, Rgb};
    let mut img = ImageBuffer::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let v = ((x * 255 / w) + seed) as u8;
            img.put_pixel(x, y, Rgb([v, v, 255 - v]));
        }
    }
    let path = std::env::temp_dir().join(format!("dedup-img-test-{}-{}-{}.png", std::process::id(), seed, w));
    img.save(&path).unwrap();
    path
}

fn solid_png(w: u32, h: u32, gray: u8) -> PathBuf {
    use image::{ImageBuffer, Luma};
    let img = ImageBuffer::from_pixel(w, h, Luma([gray]));
    let path = std::env::temp_dir().join(format!("dedup-img-solid-{}-{}.png", std::process::id(), gray));
    img.save(&path).unwrap();
    path
}

#[test]
fn similar_images_found() {
    let dir = test_dir("images");
    // 同内容不同尺寸的两张图（相似），一张纯色完全不同（不相似）
    let p1 = gradient_png(100, 100, 1);
    let p2 = gradient_png(200, 200, 1);
    let p3 = solid_png(100, 100, 128);
    // 拷贝进扫描目录
    let f1 = write(&dir, "a.png", &std::fs::read(&p1).unwrap());
    let f2 = write(&dir, "sub/b.png", &std::fs::read(&p2).unwrap());
    let f3 = write(&dir, "c.png", &std::fs::read(&p3).unwrap());
    let _ = std::fs::remove_file(&p1);
    let _ = std::fs::remove_file(&p2);
    let _ = std::fs::remove_file(&p3);

    let cancel = AtomicBool::new(false);
    let mut opts = base_options(vec![dir.to_string_lossy().into_owned()]);
    opts.similar_images = true;
    opts.image_threshold = 10;

    let result = dedup_core::run_scan(&opts, None, Some(&cancel)).unwrap();
    let similar: Vec<_> = result.groups.iter().filter(|g| g.kind == dedup_core::GroupKind::SimilarImage).collect();
    assert_eq!(similar.len(), 1, "应找到 1 组相似图片，实际 {}", similar.len());
    let paths: Vec<&str> = similar[0].files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.iter().any(|p| p.ends_with("a.png")), "a.png 应在组内");
    assert!(paths.iter().any(|p| p.ends_with("b.png")), "b.png 应在组内");
    assert!(!paths.iter().any(|p| p.ends_with("c.png")), "纯色 c.png 不应入组");
    let _ = f1;
    let _ = f2;
    let _ = f3;
}

#[test]
fn cleanup_tools_collected() {
    let dir = test_dir("tools");
    write(&dir, "big.bin", &vec![0u8; 20000]);
    write(&dir, "small.bin", &vec![0u8; 10]);
    write(&dir, "temp.tmp", b"tmp file");
    write(&dir, "keep.txt", b"keep me");
    std::fs::create_dir_all(dir.join("empty_dir")).unwrap();
    std::fs::create_dir_all(dir.join("not_empty")).unwrap();
    write(&dir, "not_empty/x.txt", b"x");

    let cancel = AtomicBool::new(false);
    let mut opts = base_options(vec![dir.to_string_lossy().into_owned()]);
    opts.tool_empty_folders = true;
    opts.tool_big_files = true;
    opts.tool_big_files_count = 5;
    opts.tool_temp_files = true;
    opts.music_dedup = true; // 无音频文件，不应崩溃
    opts.similar_videos = true; // 无视频文件，不应崩溃

    let result = dedup_core::run_scan(&opts, None, Some(&cancel)).unwrap();
    let kinds: Vec<_> = result.tools.iter().map(|t| t.kind).collect();
    assert!(kinds.contains(&dedup_core::ToolKind::EmptyFolder), "应报告空文件夹");
    assert!(kinds.contains(&dedup_core::ToolKind::BigFile), "应报告大文件");
    assert!(kinds.contains(&dedup_core::ToolKind::TempFile), "应报告临时文件");
    let empty = result.tools.iter().find(|t| t.kind == dedup_core::ToolKind::EmptyFolder).unwrap();
    assert!(empty.path.ends_with("empty_dir"), "应为 empty_dir，实际 {}", empty.path);
    let big = result.tools.iter().find(|t| t.kind == dedup_core::ToolKind::BigFile).unwrap();
    assert!(big.path.ends_with("big.bin"), "最大文件应为 big.bin");
    // 测试目录位于 %TEMP%，"临时目录"规则会命中全部文件；断言 temp.tmp 在其中即可
    let temps: Vec<&str> = result
        .tools
        .iter()
        .filter(|t| t.kind == dedup_core::ToolKind::TempFile)
        .map(|t| t.path.as_str())
        .collect();
    assert!(temps.iter().any(|p| p.ends_with("temp.tmp")), "temp.tmp 应被识别为临时文件");
    // 空文件夹删除操作
    let req_paths = vec![empty.path.clone()];
    let res = dedup_core::action::remove_empty_dirs(&req_paths);
    assert!(res[0].ok, "空文件夹应删除成功: {}", res[0].message);
    assert!(!std::path::Path::new(&empty.path).exists());
    // 非空目录不应被删除
    let req2 = vec![dir.join("not_empty").to_string_lossy().into_owned()];
    let res2 = dedup_core::action::remove_empty_dirs(&req2);
    assert!(!res2[0].ok, "非空目录应拒绝删除");
}
