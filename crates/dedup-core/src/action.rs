//! 文件操作：回收站 / 永久删除 / 硬链接替换 / 移动 / 复制
//!
//! 安全机制：
//! - 操作前校验文件仍存在且 (size, mtime) 与扫描时一致（TOCTTOU），不一致则跳过并报告
//! - 硬链接替换采用"先建临时链接 → 删除原文件 → 重命名"顺序，任一步失败都不丢数据
//! - 永久删除前清除只读属性

use crate::models::{ActionItem, ActionKind, ActionRequest, ActionResult, BatchActionRequest, FileEntry};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// 操作前校验：文件仍存在且未被修改
fn verify_unchanged(entry: &FileEntry) -> Result<(), String> {
    let meta = std::fs::metadata(&entry.path).map_err(|e| format!("无法访问文件: {e}"))?;
    if !meta.is_file() {
        return Err("不再是普通文件".to_string());
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if meta.len() != entry.size {
        return Err(format!("大小已变化（扫描时 {}，现在 {}），已跳过", entry.size, meta.len()));
    }
    if modified != entry.modified {
        return Err("修改时间已变化，已跳过".to_string());
    }
    Ok(())
}

fn clear_readonly(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(path, perms);
    }
}

fn dest_path(dest_dir: &str, src: &Path) -> PathBuf {
    let name = src.file_name().unwrap_or_default();
    Path::new(dest_dir).join(name)
}

/// 硬链接替换：创建临时链接 → 删除原文件 → 重命名，任一步失败都不丢数据
fn hardlink_replace(p: &Path, reference: &str) -> Result<(), String> {
    let ref_path = Path::new(reference);
    if !ref_path.exists() {
        return Err("参考文件不存在，无法创建硬链接".to_string());
    }
    let tmp = p.with_extension(format!(
        "{}_dedup_tmp",
        p.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    // 1. 创建指向参考文件的临时硬链接
    std::fs::hard_link(ref_path, &tmp).map_err(|e| format!("创建硬链接失败: {e}"))?;
    // 2. 删除原文件
    clear_readonly(p);
    if let Err(e) = std::fs::remove_file(p) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("删除原文件失败（已回滚临时链接）: {e}"));
    }
    // 3. 重命名临时链接为原路径
    std::fs::rename(&tmp, p).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("重命名失败（已清理临时链接）: {e}")
    })
}

/// 移动到目标目录：同卷直接改名，跨卷复制后删除
fn move_file(p: &Path, dest_dir: Option<&str>) -> Result<(), String> {
    let dest = dest_dir.ok_or_else(|| "未指定目标目录".to_string())?;
    let target = dest_path(dest, p);
    match std::fs::rename(p, &target) {
        Ok(_) => Ok(()),
        Err(_) => {
            std::fs::copy(p, &target).map_err(|e| format!("复制失败: {e}"))?;
            clear_readonly(p);
            std::fs::remove_file(p).map_err(|e| {
                let _ = std::fs::remove_file(&target);
                format!("删除源文件失败（已回滚复制）: {e}")
            })
        }
    }
}

/// 复制到目标目录
fn copy_file(p: &Path, dest_dir: Option<&str>) -> Result<(), String> {
    let dest = dest_dir.ok_or_else(|| "未指定目标目录".to_string())?;
    let target = dest_path(dest, p);
    std::fs::copy(p, &target).map_err(|e| format!("复制失败: {e}"))?;
    Ok(())
}

/// 执行单个文件的一种操作（item 携带其所属组的参考文件）
fn apply_one(kind: ActionKind, item: &ActionItem, dest_dir: Option<&str>) -> ActionResult {
    let entry = &item.file;
    let path = entry.path.clone();
    let fail = |msg: String| ActionResult { path: path.clone(), ok: false, message: msg };

    if let Err(e) = verify_unchanged(entry) {
        return fail(e);
    }

    let p = Path::new(&entry.path);
    let result: Result<(), String> = match kind {
        ActionKind::Trash => {
            trash::delete(p).map_err(|e| format!("移入回收站失败: {e}"))
        }
        ActionKind::Delete => {
            clear_readonly(p);
            std::fs::remove_file(p).map_err(|e| format!("删除失败: {e}"))
        }
        ActionKind::Hardlink => hardlink_replace(p, &item.reference),
        ActionKind::Move => move_file(p, dest_dir),
        ActionKind::Copy => copy_file(p, dest_dir),
    };

    match result {
        Ok(()) => ActionResult { path, ok: true, message: "成功".to_string() },
        Err(e) => fail(e),
    }
}

/// 批量执行操作（并行，支持跨组文件，每个文件带自己的参考），返回每个文件的结果
pub fn apply_batch_action(req: &BatchActionRequest) -> Vec<ActionResult> {
    let kind = req.kind;
    let dest_dir = req.dest_dir.clone();
    req.items
        .par_iter()
        .map(|item| apply_one(kind, item, dest_dir.as_deref()))
        .collect()
}

/// 兼容旧接口：整组文件共享一个参考文件
pub fn apply_action(req: &ActionRequest) -> Vec<ActionResult> {
    let items: Vec<ActionItem> = req
        .items
        .iter()
        .map(|f| ActionItem {
            file: f.clone(),
            reference: req.reference.clone(),
        })
        .collect();
    apply_batch_action(&BatchActionRequest {
        kind: req.kind,
        items,
        dest_dir: req.dest_dir.clone(),
    })
}

/// 删除空文件夹（操作前校验目录仍存在且确为空）
pub fn remove_empty_dirs(paths: &[String]) -> Vec<ActionResult> {
    paths
        .par_iter()
        .map(|p| {
            let path = p.clone();
            let fail = |msg: String| ActionResult { path: path.clone(), ok: false, message: msg };
            let dir = Path::new(p);
            if !dir.exists() {
                return fail("目录已不存在".to_string());
            }
            if !dir.is_dir() {
                return fail("不再是目录".to_string());
            }
            // 校验确实为空（忽略 desktop.ini 等系统文件）
            let mut has_real = false;
            if let Ok(rd) = std::fs::read_dir(dir) {
                for entry in rd.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let ign = ["desktop.ini", "thumbs.db", ".ds_store"]
                        .iter()
                        .any(|x| x.eq_ignore_ascii_case(&name));
                    if !ign {
                        has_real = true;
                        break;
                    }
                }
            }
            if has_real {
                return fail("目录不为空，已跳过".to_string());
            }
            match std::fs::remove_dir(dir) {
                Ok(()) => ActionResult { path, ok: true, message: "已删除空文件夹".to_string() },
                Err(e) => fail(format!("删除失败: {e}")),
            }
        })
        .collect()
}
