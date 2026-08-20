// Tauri 命令与类型定义桥接
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'

// ---------- 类型 ----------
export type KeepStrategy = 'keep_newest' | 'keep_oldest' | 'keep_largest' | 'keep_first'
export type ActionKind = 'trash' | 'delete' | 'hardlink' | 'move' | 'copy'
export type GroupKind = 'exact' | 'fuzzy_name' | 'similar_image'

export interface ScanOptions {
  paths: string[]
  recursive: boolean
  min_size: number
  max_size: number
  only_extensions: string[]
  exclude_extensions: string[]
  exclude_paths: string[]
  follow_symlinks: boolean
  max_depth: number | null
  keep_strategy: KeepStrategy
  use_cache: boolean
  cache_path: string
  fuzzy_filename: boolean
  fuzzy_threshold: number
  fuzzy_same_dir_only: boolean
  similar_images: boolean
  image_threshold: number
}

export interface FileEntry {
  path: string
  size: number
  modified: number
  created: number
}

export interface DuplicateGroup {
  files: FileEntry[]
  kind: GroupKind
  file_size: number
  reclaimable: number
}

export interface ScanResult {
  groups: DuplicateGroup[]
  scanned_files: number
  scanned_bytes: number
  cache_hits: number
  elapsed_ms: number
}

export interface ProgressUpdate {
  phase: string
  done: number
  total: number
  message: string
}

// 批量操作：每个文件携带其所属组的参考文件（支持跨组批量）
export interface ActionItem {
  file: FileEntry
  reference: string
}

export interface BatchActionRequest {
  kind: ActionKind
  items: ActionItem[]
  dest_dir: string | null
}

export interface ActionResult {
  path: string
  ok: boolean
  message: string
}

// ---------- 命令 ----------
export function runScan(options: ScanOptions): Promise<ScanResult> {
  return invoke('run_scan', { options })
}
export function cancelScan(): Promise<void> {
  return invoke('cancel_scan')
}
export function applyAction(req: BatchActionRequest): Promise<ActionResult[]> {
  return invoke('apply_action', { req })
}
export function clearCache(path: string): Promise<number> {
  return invoke('clear_cache', { path })
}
export function getCacheStats(path: string): Promise<{ entries: number }> {
  return invoke('get_cache_stats', { path })
}
export function defaultCachePath(): Promise<string> {
  return invoke('default_cache_path')
}
export function onScanProgress(cb: (p: ProgressUpdate) => void): Promise<() => void> {
  return listen<ProgressUpdate>('scan-progress', (e) => cb(e.payload))
}
export function pickDirectory(multiple = false): Promise<string | string[] | null> {
  return open({ directory: true, multiple })
}

// ---------- 格式化工具 ----------
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB', 'TB', 'PB']
  let v = n
  let i = -1
  do {
    v /= 1024
    i++
  } while (v >= 1024 && i < units.length - 1)
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`
}

export function formatDate(unixSec: number): string {
  if (!unixSec) return '-'
  const d = new Date(unixSec * 1000)
  const pad = (x: number) => String(x).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}
