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
    /// 是否启用文件名模糊匹配
    #[serde(default)]
    pub fuzzy_filename: bool,
    /// 文件名模糊匹配相似度阈值（0-100，dupeGuru 式词相似度）
    #[serde(default = "default_fuzzy_threshold")]
    pub fuzzy_threshold: u8,
    /// 模糊匹配仅在同一目录内比较（默认 true）
    #[serde(default = "default_true")]
    pub fuzzy_same_dir_only: bool,
    /// 是否启用相似图片查找
    #[serde(default)]
    pub similar_images: bool,
    /// 相似图片感知哈希汉明距离阈值（0-64）
    #[serde(default = "default_image_threshold")]
    pub image_threshold: u32,
    // ---- M3：音乐去重 ----
    /// 是否启用音乐标签去重
    #[serde(default)]
    pub music_dedup: bool,
    /// 音乐标签匹配相似度阈值（0-100）
    #[serde(default = "default_fuzzy_threshold")]
    pub music_threshold: u8,
    // ---- M3：附加清理工具 ----
    /// 是否查找空文件夹
    #[serde(default)]
    pub tool_empty_folders: bool,
    /// 是否列出大文件
    #[serde(default)]
    pub tool_big_files: bool,
    /// 大文件列表数量上限
    #[serde(default = "default_big_count")]
    pub tool_big_files_count: usize,
    /// 是否列出临时文件
    #[serde(default)]
    pub tool_temp_files: bool,
    // ---- M4：相似视频 ----
    /// 是否启用相似视频查找
    #[serde(default)]
    pub similar_videos: bool,
    /// 视频帧感知哈希汉明距离阈值（0-64）
    #[serde(default = "default_video_threshold")]
    pub video_threshold: u32,
    /// ffmpeg 可执行文件路径（空 = 自动查找应用目录/PATH）
    #[serde(default)]
    pub ffmpeg_path: String,
}

fn default_fuzzy_threshold() -> u8 {
    80
}
fn default_image_threshold() -> u32 {
    10
}
fn default_video_threshold() -> u32 {
    10
}
fn default_true() -> bool {
    true
}
fn default_big_count() -> usize {
    50
}

/// 分组类型：精确内容重复 / 文件名模糊匹配 / 相似图片 / 音乐 / 相似视频
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    /// 内容完全相同的重复文件
    Exact,
    /// 文件名模糊匹配（需人工确认）
    FuzzyName,
    /// 感知哈希相似图片（需人工确认）
    SimilarImage,
    /// 音乐标签去重（需人工确认）
    MusicTag,
    /// 相似视频（需人工确认）
    SimilarVideo,
}

/// 附加清理工具的类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// 空文件夹
    EmptyFolder,
    /// 大文件
    BigFile,
    /// 临时文件
    TempFile,
}

/// 附加清理工具的结果条目（非重复分组）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolItem {
    pub kind: ToolKind,
    /// 完整路径
    pub path: String,
    /// 大小（空文件夹为 0）
    pub size: u64,
    /// 修改时间（Unix 秒，空文件夹为 0）
    pub modified: u64,
    /// 创建时间（Unix 秒，可能为 0）
    pub created: u64,
    /// 说明文字
    pub detail: String,
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
    /// 分组类型
    pub kind: GroupKind,
    /// 组内第一个文件（参考文件）的大小
    pub file_size: u64,
    /// 可释放空间 = 组内除参考文件外所有文件大小之和
    pub reclaimable: u64,
}

impl DuplicateGroup {
    /// 构造分组；调用方需保证 `files` 已按保留策略排序（files[0] 为参考文件）
    pub fn new(files: Vec<FileEntry>, kind: GroupKind) -> Self {
        let file_size = files.first().map(|f| f.size).unwrap_or(0);
        let reclaimable = files.iter().skip(1).map(|f| f.size).sum();
        Self { files, kind, file_size, reclaimable }
    }
}

/// 扫描结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub groups: Vec<DuplicateGroup>,
    /// 附加清理工具结果（空文件夹/大文件/临时文件）
    #[serde(default)]
    pub tools: Vec<ToolItem>,
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
