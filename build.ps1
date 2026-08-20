# 文件去重 - 构建辅助脚本（本机开发用）
# 用法：
#   .\build.ps1 dev          # 开发模式（tauri dev，自动启动前端 + 应用）
#   .\build.ps1 build        # 编译调试版应用（Rust 侧）
#   .\build.ps1 test         # 运行核心引擎测试
#   .\build.ps1 bundle       # 打包安装程序（NSIS）
#   .\build.ps1 frontend     # 仅构建前端
param([string]$Task = 'build')

$ErrorActionPreference = 'Stop'
$env:PATH = "$env:USERPROFILE\.cargo\bin;D:\DSH\文件去重\.toolchain\w64devkit\bin;$env:PATH"
$env:CC = 'D:\DSH\文件去重\.toolchain\w64devkit\bin\gcc.exe'
# 构建产物放到纯 ASCII 路径，规避 Windows 下 GNU 链接器对中文路径的编码问题
# （可用 FILEDEDUP_TARGET_DIR 环境变量覆盖）
if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = if ($env:FILEDEDUP_TARGET_DIR) { $env:FILEDEDUP_TARGET_DIR } else { 'D:/DSH/filededup-build' }
}

switch ($Task) {
    'dev'      { npm.cmd run tauri dev }
    'build'    { cargo build -p filededup }
    'test'     { cargo test -p dedup-core }
    'bundle'   { npm.cmd run tauri build }
    'frontend' { npm.cmd run build }
    default    { Write-Host "未知任务: $Task" }
}
