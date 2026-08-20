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
