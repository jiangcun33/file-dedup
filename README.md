# 文件去重（File Dedup）

一个安全优先的 Windows 文件去重工具：通过内容哈希精确查找重复文件，提供批量选择与跨组批量清理能力。

- **前端**：Tauri 2 + Vue 3 + Element Plus + Vite
- **后端**：Rust（`dedup-core` 核心引擎，无 UI 依赖，可独立测试）
- **许可证**：MIT

## ✨ 功能特性

- ✅ **精确内容去重**：三级漏斗（大小 → xxh3 部分哈希 → blake3 全哈希）+ 多线程并行
- ✅ **文件名模糊匹配**：dupeGuru 式词相似度算法（中文分词、相似词、阈值可调、仅同目录选项）
- ✅ **相似图片查找**：自研 pHash（DCT 感知哈希）+ BK 树近邻匹配 + 图片哈希缓存
- ✅ **哈希缓存**：SQLite 持久化，二次扫描几乎瞬时
- ✅ **过滤**：多目录、递归深度、大小范围、扩展名包含/排除、路径排除
- ✅ **保留策略**：每组自动保留一个参考文件（最旧 / 最新 / 最大 / 扫描序）
- ✅ **文件操作**：移到回收站 / 永久删除（二次确认）/ 硬链接替换（安全三步）/ 移动 / 复制
- ✅ **批量快速选择**：全选 / 反选 / 取消；按文件名长度、路径深度、创建/修改时间、文件名关键词勾选
- ✅ **批量设置保留**：按条件重排每组保留文件（对标 dupeGuru / jdupes 的排序策略）
- ✅ **跨组批量操作**：对勾选副本统一执行；硬链接逐文件关联本组保留文件
- ✅ **分组类型标记与筛选**：精确重复 / 文件名模糊 / 相似图片，模糊与相似结果执行操作前需人工确认
- ✅ **安全机制**：扫描全程只读、操作前 size+mtime 防篡改校验（TOCTTOU）、参考文件永不被勾选

## 🚀 快速开始（Windows）

### 前置要求

| 依赖 | 说明 |
|---|---|
| [Rust](https://rustup.rs/) | stable，MSVC 或 GNU 工具链均可 |
| [Node.js](https://nodejs.org/) | 18+（含 npm） |
| [WebView2 运行时](https://developer.microsoft.com/microsoft-edge/webview2/) | Win10/11 一般已内置 |

### 构建运行

```bash
npm install
npm run tauri dev      # 开发模式
npm run tauri build    # 打包 NSIS 安装包（产物在 src-tauri/target/release/bundle/）
```

### 核心引擎独立测试

```bash
cargo test -p dedup-core
```

## 📁 目录结构

```
├── crates/dedup-core/    # 核心引擎（扫描/哈希/分组/缓存/操作，无 UI 依赖）
│   ├── src/scan.rs       # 目录遍历
│   ├── src/hash.rs       # 三级漏斗哈希与分组
│   ├── src/cache.rs      # SQLite 哈希缓存
│   ├── src/action.rs     # 文件操作与安全校验
│   └── tests/            # 集成测试（含跨组批量操作）
├── src-tauri/            # Tauri 应用（命令桥接层）
├── src/                  # Vue 3 前端（扫描页 / 结果页）
├── docs/                 # 调研报告与技术方案
└── icon/                 # 应用图标源文件
```

## 🛠 开发备忘

- **GNU 工具链构建**：Tauri 在 GNU 下无法静态链接 `WebView2Loader.dll`，仓库已将该 DLL 作为 bundle 资源随安装包分发（`src-tauri/resources/`）。
- **中文路径问题**：Windows 下 GNU 链接器（ld/windres）对含中文的路径处理不佳，建议将源码放在 ASCII 路径，或把 `CARGO_TARGET_DIR` 指向 ASCII 目录后构建。

## 🗺 路线图

- [x] M0：精确去重 MVP（扫描 / 分组 / 展示 / 回收站 / 保留策略）
- [x] M1：批量选择与跨组批量操作
- [x] M2：文件名模糊匹配、相似图片（感知哈希）
- [ ] M3：音乐标签去重、空文件夹 / 大文件 / 临时文件工具
- [ ] M4：相似视频、性能优化、发布

## 📄 文档

- [调研报告：开源去重项目功能汇总](research/文件去重软件调研报告.md)
- [技术方案设计](docs/技术方案设计.md)

## 📜 许可证

[MIT](LICENSE)
