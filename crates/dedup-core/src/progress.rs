//! 进度推送与取消检查

use crate::models::ProgressUpdate;
use std::sync::atomic::{AtomicBool, Ordering};

/// 向 channel 发送一条进度（channel 可能已关闭，忽略错误）
pub fn send(tx: Option<&crossbeam_channel::Sender<ProgressUpdate>>, phase: &str, done: u64, total: u64, message: &str) {
    if let Some(tx) = tx {
        let _ = tx.send(ProgressUpdate {
            phase: phase.to_string(),
            done,
            total,
            message: message.to_string(),
        });
    }
}

/// 检查是否被要求取消
pub fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false)
}
