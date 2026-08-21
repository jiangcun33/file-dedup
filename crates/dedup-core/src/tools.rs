//! 附加清理工具：空文件夹 / 大文件 / 临时文件

use crate::models::{FileEntry, ToolItem, ToolKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 临时文件扩展名（保守集合）
const TEMP_EXTS: &[&str] = &["tmp", "temp", "crdownload", "part", "partial", "download", "swp", "lock", "dmp", "bak~", "cache"];
/// 判断空文件夹时忽略的文件（系统自动生成）
const IGNORABLE_FILES: &[&str] = &["desktop.ini", "thumbs.db", ".ds_store"];

fn is_ignorable(name: &str) -> bool {
    IGNORABLE_FILES.iter().any(|x| x.eq_ignore_ascii_case(name))
}

/// 判断目录是否为空（忽略系统自动生成文件；只含空子目录也算空）
fn dir_is_empty(dir: &Path, cache: &mut HashMap<PathBuf, bool>) -> bool {
    if let Some(&v) = cache.get(dir) {
        return v;
    }
    let mut empty = true;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_ignorable(&name) {
                continue;
            }
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                if !dir_is_empty(&path, cache) {
                    empty = false;
                    break;
                }
            } else if ft.is_file() || ft.is_symlink() {
                empty = false;
                break;
            }
        }
    }
    cache.insert(dir.to_path_buf(), empty);
    empty
}

/// 查找空文件夹（仅顶层为空的目录，且不递归进入非空目录）
pub fn find_empty_folders(roots: &[String], recursive: bool) -> Vec<ToolItem> {
    let mut out = Vec::new();
    let mut cache = HashMap::new();
    for root in roots {
        let root_path = Path::new(root);
        if !root_path.is_dir() {
            continue;
        }
        if dir_is_empty(root_path, &mut cache) {
            out.push(ToolItem {
                kind: ToolKind::EmptyFolder,
                path: root.to_string(),
                size: 0,
                modified: 0,
                created: 0,
                detail: "空文件夹".to_string(),
            });
            continue; // 根目录为空，无需递归
        }
        if !recursive {
            continue;
        }
        // 递归收集所有空目录
        let mut stack = vec![root_path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                let mut subdirs = Vec::new();
                for entry in rd.flatten() {
                    let ft = match entry.file_type() {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    if ft.is_dir() {
                        subdirs.push(entry.path());
                    }
                }
                for sub in subdirs {
                    if dir_is_empty(&sub, &mut cache) {
                        out.push(ToolItem {
                            kind: ToolKind::EmptyFolder,
                            path: sub.to_string_lossy().into_owned(),
                            size: 0,
                            modified: 0,
                            created: 0,
                            detail: "空文件夹".to_string(),
                        });
                    } else if recursive {
                        stack.push(sub);
                    }
                }
            }
        }
    }
    out
}

/// 列出最大的 N 个文件
pub fn find_big_files(entries: &[FileEntry], count: usize) -> Vec<ToolItem> {
    let mut sorted: Vec<&FileEntry> = entries.iter().filter(|e| e.size > 0).collect();
    sorted.sort_by(|a, b| b.size.cmp(&a.size));
    sorted
        .iter()
        .take(count)
        .map(|e| ToolItem {
            kind: ToolKind::BigFile,
            path: e.path.clone(),
            size: e.size,
            modified: e.modified,
            created: e.created,
            detail: format!("大文件（{:.1} MB）", e.size as f64 / 1048576.0),
        })
        .collect()
}

fn ext_of(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
}

/// 列出临时文件（扩展名/特征路径匹配）
pub fn find_temp_files(entries: &[FileEntry]) -> Vec<ToolItem> {
    let mut out = Vec::new();
    for e in entries {
        let name = e
            .path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_lowercase();
        let ext = ext_of(&e.path);
        let is_ext = ext
            .as_deref()
            .map(|x| TEMP_EXTS.iter().any(|t| *t == x))
            .unwrap_or(false);
        let is_tilde = name.starts_with("~$");
        let in_temp_dir = e.path.contains("\\Temp\\") || e.path.contains("/Temp/") || e.path.contains("\\TEMP\\");
        if is_ext || is_tilde || in_temp_dir {
            let why = if is_tilde {
                "Office 临时锁定文件".to_string()
            } else if in_temp_dir {
                "位于临时目录".to_string()
            } else {
                format!("临时文件扩展名 .{}", ext.unwrap_or_default())
            };
            out.push(ToolItem {
                kind: ToolKind::TempFile,
                path: e.path.clone(),
                size: e.size,
                modified: e.modified,
                created: e.created,
                detail: why,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_folder_detection() {
        let base = std::env::temp_dir().join(format!("dedup-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("a/empty1")).unwrap();
        std::fs::create_dir_all(base.join("a/empty2")).unwrap();
        std::fs::create_dir_all(base.join("b")).unwrap();
        std::fs::write(base.join("b/file.txt"), b"x").unwrap();
        let items = find_empty_folders(&[base.to_string_lossy().into_owned()], true);
        let empties: Vec<&str> = items.iter().map(|i| i.path.as_str()).collect();
        // 只报告最顶层空目录（a 只含空子目录 → a 为空；empty1/empty2 隐含在 a 内）
        assert!(empties.iter().any(|p| p.ends_with("a")), "a（只含空目录）应为空文件夹");
        assert!(!empties.iter().any(|p| p.ends_with("empty1")), "empty1 已隐含在 a 内，不单独报告");
        assert!(!empties.iter().any(|p| p.ends_with("b")), "b 有文件不应为空");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn big_files_sorted() {
        let base = std::env::temp_dir().join(format!("dedup-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mk = |name: &str, size: usize| {
            let p = base.join(name);
            std::fs::write(&p, vec![0u8; size]).unwrap();
        };
        mk("small", 100);
        mk("big", 10000);
        mk("mid", 1000);
        let entries: Vec<FileEntry> = std::fs::read_dir(&base)
            .unwrap()
            .flatten()
            .map(|e| {
                let m = e.metadata().unwrap();
                FileEntry::from_meta(e.path(), &m)
            })
            .collect();
        let items = find_big_files(&entries, 2);
        assert_eq!(items.len(), 2);
        assert!(items[0].path.ends_with("big"), "最大的应排第一");
        let _ = std::fs::remove_dir_all(&base);
    }
}
