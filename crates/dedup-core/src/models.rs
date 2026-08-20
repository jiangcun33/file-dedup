//! 数据结构定义（与前端通过 serde JSON 交互）

use serde::{Deserialize, Serialize};

/// 扫描选项
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanOptions {
    /// 待扫描的根目录列表
    pub paths: Vec<String>,
    /// 是否递归子目录
    pub recursive: bool,
    /// 最小文件大小（字节），0 表示不限
    pub min_size: u64,
    /// 最大文件大小（字节），0 表示不限
    pub max_size: u64,
    /// 仅扫描这些扩展名（小写、不带点），空表示不限
    pub only_extensions: Vec<String>,
    /// 排除这些扩展名
    pub exclude_extensions: Vec<String>,
    /// 路径中包含这些子串的条目将被排除（用于排除目录）
    pub exclude_paths: Vec<String>,
    /// 是否跟随符号链接
    pub follow_symlinks: bool,
    /// 最大递归深度（None 表示不限）
    pub max_depth: Option<usize>,
    /// 每组保留（参考文件）的策略
    pub keep_strategy: KeepStrategy,
    /// 是否使用哈希缓存
    pub use_cache: bool,
    /// 缓存数据库文件路径
    pub cache_path: String,
}

/// 保留策略：决定每组中谁是"参考文件"（不会被删除）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeepStrategy {
    /// 保留最新（修改时间最新者当参考）
    KeepNewest,
    /// 保留最旧（默认）
    #[default]
    KeepOldest,
    /// 保留最大
    KeepLargest,
    /// 保留扫描顺序第一个
    KeepFirst,
}

/// 单个文件条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// 完整路径
    pub path: String,
    /// 文件大小（字节）
    pub size: u64,
    /// 修改时间（Unix 秒）
    pub modified: u64,
    /// 创建时间（Unix 秒，可能为 0）
    pub created: u64,
}

impl FileEntry {
    pub fn from_meta(path: std::path::PathBuf, meta: &std::fs::Metadata) -> Self {
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let created = meta
            .created()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            path: path.to_string_lossy().into_owned(),
            size: meta.len(),
            modified,
            created,
        }
    }
}

/// 一组重复文件；`files[0]` 为参考文件（保留对象）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub files: Vec<FileEntry>,
    /// 组内单文件大小（组内所有文件同大小）
    pub file_size: u64,
    /// 可释放空间 = (n-1) * file_size
    pub reclaimable: u64,
}

impl DuplicateGroup {
    /// 构造分组；调用方需保证 `files` 已按保留策略排序（files[0] 为参考文件）
    pub fn new(files: Vec<FileEntry>) -> Self {
        let file_size = files.first().map(|f| f.size).unwrap_or(0);
        let reclaimable = if files.len() > 1 { file_size * (files.len() as u64 - 1) } else { 0 };
        Self { files, file_size, reclaimable }
    }
}

/// 扫描结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub groups: Vec<DuplicateGroup>,
    pub scanned_files: u64,
    pub scanned_bytes: u64,
    pub cache_hits: u64,
    pub elapsed_ms: u128,
}

/// 进度更新（经 crossbeam channel 推送给调用方）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressUpdate {
    pub phase: String,
    pub done: u64,
    pub total: u64,
    pub message: String,
}

/// 操作类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// 移到回收站
    Trash,
    /// 永久删除
    Delete,
    /// 用硬链接替换为参考文件
    Hardlink,
    /// 移动到指定目录
    Move,
    /// 复制到指定目录
    Copy,
}

/// 操作请求：对一组文件执行一种操作
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRequest {
    pub kind: ActionKind,
    /// 待操作的文件（含扫描时的元数据，用于操作前校验）
    pub items: Vec<FileEntry>,
    /// 参考文件路径（Hardlink 时使用）
    pub reference: String,
    /// 目标目录（Move/Copy 时使用）
    pub dest_dir: Option<String>,
}

/// 批量操作中的单个文件（带其所属组的参考文件，支持跨组批量操作）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionItem {
    pub file: FileEntry,
    /// 该文件所属重复组的参考文件路径（Hardlink 时使用）
    pub reference: String,
}

/// 批量操作请求：对多个文件（可能跨组）执行一种操作
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchActionRequest {
    pub kind: ActionKind,
    pub items: Vec<ActionItem>,
    /// 目标目录（Move/Copy 时使用）
    pub dest_dir: Option<String>,
}

/// 单个文件的操作结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub path: String,
    pub ok: bool,
    pub message: String,
}
