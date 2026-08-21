<script setup lang="ts">
import { ref, computed, reactive } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  applyAction,
  removeEmptyDirs,
  formatBytes,
  formatDate,
  pickDirectory,
  type ScanResult,
  type DuplicateGroup,
  type FileEntry,
  type ActionResult,
  type ActionKind,
  type ActionItem,
  type GroupKind,
  type ToolItem,
} from '../api'

const props = defineProps<{ result: ScanResult }>()
const emit = defineEmits<{ (e: 'back'): void }>()

const groups = ref<DuplicateGroup[]>(props.result.groups.map((g) => ({ ...g, files: g.files.map((f) => ({ ...f })) })))
const tools = ref<ToolItem[]>(props.result.tools.map((t) => ({ ...t })))
const activeNames = ref<string[]>(groups.value.map((_, i) => String(i)))
const pending = ref(false)

// 分组类型筛选（显示层）
const kindFilter = ref<'all' | GroupKind>('all')
const visibleGroups = computed(() =>
  groups.value
    .map((g, i) => ({ g, i }))
    .filter(({ g }) => kindFilter.value === 'all' || g.kind === kindFilter.value),
)
function kindInfo(k: GroupKind): { text: string; type: 'primary' | 'warning' | 'success' | 'info' | 'danger' } {
  switch (k) {
    case 'exact':
      return { text: '精确重复', type: 'primary' }
    case 'fuzzy_name':
      return { text: '文件名模糊', type: 'warning' }
    case 'similar_image':
      return { text: '相似图片', type: 'success' }
    case 'music_tag':
      return { text: '音乐重复', type: 'info' }
    case 'similar_video':
      return { text: '相似视频', type: 'danger' }
  }
}
const TOOL_NAMES: Record<ToolItem['kind'], string> = {
  empty_folder: '空文件夹',
  big_file: '大文件',
  temp_file: '临时文件',
}

// 清理工具的勾选与操作
const checkedTools = ref<Set<string>>(new Set())
const checkedToolCount = computed(() => checkedTools.value.size)
function toggleTool(path: string, on: boolean) {
  const set = new Set(checkedTools.value)
  if (on) set.add(path)
  else set.delete(path)
  checkedTools.value = set
}
function toolItemsOfKind(kind: ToolItem['kind']): ToolItem[] {
  return tools.value.filter((t) => t.kind === kind)
}

// 每个组内勾选的副本路径（默认除参考外全勾）
const checked = ref<Record<number, Set<string>>>({})

function initChecks(g: DuplicateGroup, i: number) {
  const set = new Set<string>()
  g.files.forEach((f, idx) => {
    if (idx > 0) set.add(f.path)
  })
  checked.value[i] = set
}
groups.value.forEach((g, i) => initChecks(g, i))

const totalReclaimable = computed(() => groups.value.reduce((s, g) => s + g.reclaimable, 0))
const totalDupFiles = computed(() => groups.value.reduce((s, g) => s + Math.max(0, g.files.length - 1), 0))

// ---------- 工具函数 ----------
function fileName(p: string): string {
  const parts = p.split(/[\\/]/)
  return parts[parts.length - 1] || p
}
function pathDepth(p: string): number {
  return p.split(/[\\/]/).length - 1
}
function setChecked(i: number, set: Set<string>) {
  checked.value[i] = set
}

// 共享表格列宽：拖动任意一张表的列宽，所有表同步调整
const colWidths = reactive<Record<string, number>>({
  path: 380,
  size: 110,
  modified: 150,
  created: 150,
  detail: 200,
})
function onHeaderDragend(prop: string, newWidth: number) {
  if (prop && newWidth > 30) {
    colWidths[prop] = newWidth
  }
}

// ---------- 全局选择（仅作用于当前筛选可见的分组；保留文件也可勾选） ----------
function selectAllCopies() {
  visibleGroups.value.forEach(({ g, i }) => {
    const set = new Set<string>()
    g.files.forEach((f) => set.add(f.path))
    setChecked(i, set)
  })
}
function clearSelection() {
  visibleGroups.value.forEach(({ i }) => setChecked(i, new Set()))
}
function invertSelection() {
  visibleGroups.value.forEach(({ g, i }) => {
    const cur = checked.value[i] || new Set()
    const set = new Set<string>()
    g.files.forEach((f) => {
      if (!cur.has(f.path)) set.add(f.path)
    })
    setChecked(i, set)
  })
}

// ---------- 批量设置保留（每组按条件重排，参考文件置顶） ----------
const keepDialog = ref(false)
const keepCriterion = ref<'name_long' | 'name_short' | 'depth_deep' | 'depth_shallow' | 'created_new' | 'created_old' | 'modified_new' | 'modified_old' | 'path_under_dir' | 'path_outside_dir' | 'path_long' | 'path_short'>('modified_new')
const keepDir = ref<string | null>(null)

function isUnderDir(p: string, dir: string | null): boolean {
  if (!dir) return false
  const d = dir.toLowerCase()
  const x = p.toLowerCase()
  return x === d || x.startsWith(d + '\\') || x.startsWith(d + '/')
}
async function pickKeepDir() {
  const d = await pickDirectory(false)
  if (!d) return
  keepDir.value = d as string
}
function isKeepDirCriterion(): boolean {
  return keepCriterion.value === 'path_under_dir' || keepCriterion.value === 'path_outside_dir'
}

function applyKeepCriterion() {
  // 应用规则：先清除所有勾选 → 符合条件的文件保留（不勾选），其余文件勾选为待处理；
  // 若某组无符合条件的文件，则该组整组保留（全部不勾选）
  let kept = 0
  groups.value.forEach((g, i) => {
    if (g.files.length === 0) {
      setChecked(i, new Set())
      return
    }
    // 找出符合条件的文件（目录类=所有匹配；其他=每组最优一个）
    let matches: FileEntry[]
    if (isKeepDirCriterion()) {
      matches = g.files.filter((f) =>
        keepCriterion.value === 'path_under_dir' ? isUnderDir(f.path, keepDir.value) : !isUnderDir(f.path, keepDir.value),
      )
    } else {
      let best = 0
      for (let idx = 1; idx < g.files.length; idx++) {
        if (betterKeep(g.files[idx], g.files[best])) best = idx
      }
      matches = [g.files[best]]
    }
    const set = new Set<string>()
    if (matches.length === 0) {
      // 无符合条件的文件 → 整组保留（全部不勾选）
      kept += g.files.length
    } else {
      const matchPaths = new Set(matches.map((m) => m.path))
      g.files.forEach((f) => {
        if (!matchPaths.has(f.path)) set.add(f.path)
      })
      kept += matches.length
    }
    setChecked(i, set)
  })
  keepDialog.value = false
  ElMessage.success(`已按条件设置保留：${kept} 个文件保留，其余勾选`)
}
function betterKeep(a: FileEntry, b: FileEntry): boolean {
  switch (keepCriterion.value) {
    case 'name_long': return fileName(a.path).length > fileName(b.path).length
    case 'name_short': return fileName(a.path).length < fileName(b.path).length
    case 'depth_deep': return pathDepth(a.path) > pathDepth(b.path)
    case 'depth_shallow': return pathDepth(a.path) < pathDepth(b.path)
    case 'created_new': return a.created > b.created
    case 'created_old': return a.created < b.created
    case 'modified_new': return a.modified > b.modified
    case 'modified_old': return a.modified < b.modified
    case 'path_under_dir': return isUnderDir(a.path, keepDir.value) && !isUnderDir(b.path, keepDir.value)
    case 'path_outside_dir': return !isUnderDir(a.path, keepDir.value) && isUnderDir(b.path, keepDir.value)
    case 'path_long': return a.path.length > b.path.length
    case 'path_short': return a.path.length < b.path.length
  }
}

// ---------- 批量选择（按条件勾选副本） ----------
const selectDialog = ref(false)
const selCriterion = ref<'name_ge' | 'name_le' | 'name_length_ge' | 'name_length_le' | 'depth_ge' | 'depth_le' | 'created_before' | 'created_after' | 'modified_before' | 'modified_after' | 'name_contains' | 'name_not_contains' | 'path_under_dir' | 'path_outside_dir'>('name_ge')
const selDir = ref<string | null>(null)
const selValue = ref<number | string>(10)
const selValueText = computed(() => {
  const v = selValue.value
  return typeof v === 'number' ? String(v) : v
})

function matchesCriterion(f: FileEntry): boolean {
  const name = fileName(f.path)
  const nowDays = Math.floor(Date.now() / 1000 / 86400)
  switch (selCriterion.value) {
    case 'name_ge': return name.length >= Number(selValueText)
    case 'name_le': return name.length <= Number(selValueText)
    case 'name_length_ge': return f.path.length >= Number(selValueText)
    case 'name_length_le': return f.path.length <= Number(selValueText)
    case 'depth_ge': return pathDepth(f.path) >= Number(selValueText)
    case 'depth_le': return pathDepth(f.path) <= Number(selValueText)
    case 'created_before': return f.created > 0 && f.created < (nowDays - Number(selValueText)) * 86400
    case 'created_after': return f.created > 0 && f.created > (nowDays - Number(selValueText)) * 86400
    case 'modified_before': return f.modified > 0 && f.modified < (nowDays - Number(selValueText)) * 86400
    case 'modified_after': return f.modified > 0 && f.modified > (nowDays - Number(selValueText)) * 86400
    case 'name_contains': return name.toLowerCase().includes(selValueText.toLowerCase())
    case 'name_not_contains': return !name.toLowerCase().includes(selValueText.toLowerCase())
    case 'path_under_dir': return isUnderDir(f.path, selDir.value)
    case 'path_outside_dir': return !isUnderDir(f.path, selDir.value)
  }
}
function applyBatchSelect() {
  let matched = 0
  // 应用规则：先清除所有勾选 → 符合条件的文件勾选，其余文件保留（不勾选）；
  // 若某组无符合条件的文件，则该组整组保留（全部不勾选）
  groups.value.forEach((g, i) => {
    const set = new Set<string>()
    g.files.forEach((file) => {
      if (matchesCriterion(file)) {
        set.add(file.path)
        matched++
      }
    })
    setChecked(i, set)
  })
  selectDialog.value = false
  ElMessage.success(`已按条件勾选 ${matched} 个文件（替换了之前的勾选，其余保留）`)
}
function isTextCriterion(): boolean {
  return selCriterion.value === 'name_contains' || selCriterion.value === 'name_not_contains'
}
function isDirCriterion(): boolean {
  return selCriterion.value === 'path_under_dir' || selCriterion.value === 'path_outside_dir'
}
async function pickSelDir() {
  const d = await pickDirectory(false)
  if (!d) return
  selDir.value = d as string
}
function selUnit(): string {
  if (selCriterion.value.startsWith('name')) return '字符'
  if (selCriterion.value.startsWith('depth')) return '层'
  return '天前'
}

// ---------- 批量操作 ----------
function collectCheckedItems(): { gi: number; g: DuplicateGroup; file: FileEntry; isRef: boolean }[] {
  const items: { gi: number; g: DuplicateGroup; file: FileEntry; isRef: boolean }[] = []
  visibleGroups.value.forEach(({ g, i: gi }) => {
    const set = checked.value[gi] || new Set()
    const ref = g.files[0]?.path ?? ''
    g.files.forEach((file) => {
      if (set.has(file.path)) items.push({ gi, g, file, isRef: file.path === ref })
    })
  })
  return items
}
const checkedCount = computed(() => collectCheckedItems().length)

const ACTION_NAMES: Record<ActionKind, string> = {
  trash: '移到回收站',
  delete: '永久删除',
  hardlink: '硬链接替换',
  move: '移动到',
  copy: '复制到',
}

async function executeBatch(kind: ActionKind) {
  let items = collectCheckedItems()
  if (items.length === 0) {
    ElMessage.warning('请先勾选要处理的副本文件')
    return
  }
  const refItems = items.filter((x) => x.isRef)
  const hasRef = refItems.length > 0

  // 硬链接替换不适用于保留文件（自身即参考），自动排除
  if (kind === 'hardlink' && hasRef) {
    ElMessage.warning(`硬链接替换不适用于保留文件，已排除 ${refItems.length} 个保留文件`)
    items = items.filter((x) => !x.isRef)
    if (items.length === 0) return
  }

  // 破坏性操作包含保留文件时强警告（每组最后的副本，删除后该组无保留）
  if ((kind === 'trash' || kind === 'delete') && hasRef) {
    const refSample = refItems.slice(0, 5).map((x) => x.file.path).join('\n')
    try {
      await ElMessageBox.confirm(
        `⚠️ 勾选中包含 ${refItems.length} 个「保留文件」（各组的最后一份副本）！\n删除后这些组将没有任何保留副本，文件将被完全移除。\n\n${refSample}${refItems.length > 5 ? `\n…等 ${refItems.length} 个` : ''}\n\n确定继续吗？`,
        '警告：将删除保留文件',
        { type: 'error', confirmButtonText: '确定删除', cancelButtonText: '取消', confirmButtonClass: 'el-button--danger' },
      )
    } catch {
      return
    }
  }
  if (kind === 'delete') {
    try {
      await ElMessageBox.confirm(
        `将永久删除 ${items.length} 个文件，此操作不可恢复！\n\n${items.slice(0, 5).map((x) => x.file.path).join('\n')}${items.length > 5 ? `\n…等 ${items.length} 个文件` : ''}`,
        '永久删除确认',
        { type: 'warning', confirmButtonText: '永久删除', cancelButtonText: '取消', confirmButtonClass: 'el-button--danger' },
      )
    } catch {
      return
    }
  }
  // 模糊/相似结果需人工确认（可能不是真正重复）
  const suspect = items.filter(({ g }) => g.kind !== 'exact')
  if (suspect.length > 0 && kind !== 'delete') {
    try {
      await ElMessageBox.confirm(
        `其中 ${suspect.length} 个文件来自「${suspect[0].g.kind === 'fuzzy_name' ? '文件名模糊匹配' : '相似图片'}」结果，可能并非真正重复。\n确定对它们执行「${ACTION_NAMES[kind]}」吗？`,
        '需人工确认',
        { type: 'warning', confirmButtonText: '确认执行', cancelButtonText: '取消' },
      )
    } catch {
      return
    }
  }
  let dest_dir: string | null = null
  if (kind === 'move' || kind === 'copy') {
    const d = await pickDirectory(false)
    if (!d) return
    dest_dir = d as string
  }
  const reqItems: ActionItem[] = items.map(({ g, file }) => ({
    file,
    reference: g.files[0]?.path ?? '',
  }))
  pending.value = true
  try {
    const results: ActionResult[] = await applyAction({ kind, items: reqItems, dest_dir })
    const ok = results.filter((r) => r.ok)
    const fail = results.filter((r) => !r.ok)
    if (ok.length > 0) ElMessage.success(`${ok.length} 个文件处理成功`)
    fail.forEach((r) => ElMessage.error(r.message))
    if (ok.length > 0) {
      const okPaths = new Set(ok.map((r) => r.path))
      groups.value = groups.value
        .map((g) => ({ ...g, files: g.files.filter((f) => !okPaths.has(f.path)) }))
        .filter((g) => g.files.length > 1)
      rebuildAll()
    }
  } catch (e) {
    ElMessage.error(`操作失败：${e}`)
  } finally {
    pending.value = false
  }
}

function rebuildAll() {
  groups.value.forEach((g, i) => initChecks(g, i))
  activeNames.value = groups.value.map((_, i) => String(i))
}

// ---------- 清理工具操作 ----------
async function deleteCheckedEmptyFolders() {
  const paths = tools.value
    .filter((t) => t.kind === 'empty_folder' && checkedTools.value.has(t.path))
    .map((t) => t.path)
  if (paths.length === 0) {
    ElMessage.warning('请先勾选要删除的空文件夹')
    return
  }
  try {
    await ElMessageBox.confirm(`将删除 ${paths.length} 个空文件夹（只删除空目录，不影响任何文件）。`, '删除空文件夹', {
      type: 'warning',
      confirmButtonText: '删除',
      cancelButtonText: '取消',
    })
  } catch {
    return
  }
  pending.value = true
  try {
    const results = await removeEmptyDirs(paths)
    const ok = results.filter((r) => r.ok)
    const fail = results.filter((r) => !r.ok)
    if (ok.length > 0) ElMessage.success(`已删除 ${ok.length} 个空文件夹`)
    fail.forEach((r) => ElMessage.error(r.message))
    const okSet = new Set(ok.map((r) => r.path))
    tools.value = tools.value.filter((t) => !(t.kind === 'empty_folder' && okSet.has(t.path)))
    checkedTools.value = new Set([...checkedTools.value].filter((p) => !okSet.has(p)))
  } catch (e) {
    ElMessage.error(`操作失败：${e}`)
  } finally {
    pending.value = false
  }
}

async function toolFileAction(kind: ActionKind) {
  const items = tools.value.filter(
    (t) => (t.kind === 'big_file' || t.kind === 'temp_file') && checkedTools.value.has(t.path),
  )
  if (items.length === 0) {
    ElMessage.warning('请先勾选要处理的大文件/临时文件')
    return
  }
  if (kind === 'delete') {
    try {
      await ElMessageBox.confirm(`将永久删除 ${items.length} 个文件，此操作不可恢复！`, '永久删除确认', {
        type: 'warning',
        confirmButtonText: '永久删除',
        cancelButtonText: '取消',
        confirmButtonClass: 'el-button--danger',
      })
    } catch {
      return
    }
  }
  const reqItems: ActionItem[] = items.map((t) => ({
    file: { path: t.path, size: t.size, modified: t.modified, created: t.created } as FileEntry,
    reference: '',
  }))
  pending.value = true
  try {
    const results = await applyAction({ kind, items: reqItems, dest_dir: null })
    const ok = results.filter((r) => r.ok)
    const fail = results.filter((r) => !r.ok)
    if (ok.length > 0) ElMessage.success(`${ok.length} 个文件处理成功`)
    fail.forEach((r) => ElMessage.error(r.message))
    const okSet = new Set(ok.map((r) => r.path))
    tools.value = tools.value.filter((t) => !okSet.has(t.path))
    checkedTools.value = new Set([...checkedTools.value].filter((p) => !okSet.has(p)))
  } catch (e) {
    ElMessage.error(`操作失败：${e}`)
  } finally {
    pending.value = false
  }
}

// ---------- 组内交互 ----------
function toggleAll(g: DuplicateGroup, i: number, on: boolean) {
  const set = new Set<string>()
  if (on) g.files.forEach((f) => set.add(f.path))
  setChecked(i, set)
}
function onCheck(g: DuplicateGroup, i: number, path: string, on: boolean) {
  const set = new Set(checked.value[i] || [])
  if (on) set.add(path)
  else set.delete(path)
  setChecked(i, set)
}
function isAllChecked(g: DuplicateGroup, i: number): boolean {
  return g.files.length > 0 && (checked.value[i]?.size ?? 0) === g.files.length
}
</script>

<template>
  <div class="page results-page">
    <el-card shadow="never" class="summary">
      <div class="summary-row">
        <el-tag size="large" type="primary">{{ groups.length }} 组重复</el-tag>
        <el-tag size="large" type="danger">可释放 {{ formatBytes(totalReclaimable) }}</el-tag>
        <el-tag size="large" type="info">{{ totalDupFiles }} 个副本文件</el-tag>
        <el-tag size="large" type="info">扫描 {{ result.scanned_files }} 个文件</el-tag>
        <el-tag size="large" type="success">缓存命中 {{ result.cache_hits }}</el-tag>
        <el-tag size="large">耗时 {{ (result.elapsed_ms / 1000).toFixed(1) }}s</el-tag>
        <div class="spacer" />
        <el-button @click="emit('back')">← 重新扫描</el-button>
      </div>
    </el-card>

    <!-- 清理工具区 -->
    <el-card v-if="tools.length > 0" shadow="never" class="toolbar-card">
      <template #header>
        <div class="card-header">
          <span>🧹 清理工具（{{ tools.length }} 项，已勾选 <b class="hl">{{ checkedToolCount }}</b>）</span>
          <div>
            <el-button size="small" type="warning" plain :loading="pending" @click="deleteCheckedEmptyFolders">
              删除勾选的空文件夹
            </el-button>
            <el-button size="small" type="danger" plain :loading="pending" @click="toolFileAction('trash')">
              勾选文件移到回收站
            </el-button>
            <el-button size="small" type="danger" :loading="pending" @click="toolFileAction('delete')">
              勾选文件永久删除
            </el-button>
          </div>
        </div>
      </template>
      <el-table
        :data="tools"
        size="small"
        :row-key="(t: ToolItem) => t.path"
        border
        @header-dragend="(newWidth: number, _oldWidth: number, column: any) => onHeaderDragend(column.property, newWidth)"
      >
        <el-table-column width="48" align="center" :resizable="false">
          <template #default="{ row }">
            <el-checkbox :model-value="checkedTools.has(row.path) || false" @change="(v: boolean) => toggleTool(row.path, !!v)" />
          </template>
        </el-table-column>
        <el-table-column label="类型" prop="kind" width="110">
          <template #default="{ row }">
            <el-tag size="small" :type="row.kind === 'empty_folder' ? 'warning' : row.kind === 'big_file' ? 'danger' : 'info'">
              {{ TOOL_NAMES[row.kind as ToolItem['kind']] }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="路径" prop="path" :width="colWidths.path">
          <template #default="{ row }">
            <span class="path-cell">{{ row.path }}</span>
          </template>
        </el-table-column>
        <el-table-column label="大小" prop="size" :width="colWidths.size">
          <template #default="{ row }">{{ row.size ? formatBytes(row.size) : '-' }}</template>
        </el-table-column>
        <el-table-column label="说明" prop="detail" :width="colWidths.detail">
          <template #default="{ row }">{{ row.detail }}</template>
        </el-table-column>
      </el-table>
    </el-card>

    <el-empty v-if="groups.length === 0 && tools.length === 0" description="没有发现重复文件或可清理项" />
    <el-empty v-else-if="groups.length === 0" description="没有重复文件了（清理工具结果见上方）" :image-size="60" />

    <template v-else>
      <!-- 选择工具栏 -->
      <el-card shadow="never" class="toolbar-card">
        <div class="tool-row">
          <span class="tool-label">选择：</span>
          <el-button size="small" type="primary" plain @click="selectAllCopies">全选</el-button>
          <el-button size="small" @click="clearSelection">取消全选</el-button>
          <el-button size="small" @click="invertSelection">反选</el-button>
          <el-divider direction="vertical" />
          <el-button size="small" type="warning" plain @click="keepDialog = true">批量设置保留…</el-button>
          <el-button size="small" type="success" plain @click="selectDialog = true">按条件批量选择…</el-button>
        </div>
        <div class="tool-row">
          <span class="tool-label">批量操作（已勾选 <b class="hl">{{ checkedCount }}</b> 个副本）：</span>
          <el-button size="small" type="danger" plain :loading="pending" @click="executeBatch('trash')">移到回收站</el-button>
          <el-button size="small" type="danger" :loading="pending" @click="executeBatch('delete')">永久删除</el-button>
          <el-button size="small" type="warning" plain :loading="pending" @click="executeBatch('hardlink')">硬链接替换</el-button>
          <el-button size="small" :loading="pending" @click="executeBatch('move')">移动到…</el-button>
          <el-button size="small" :loading="pending" @click="executeBatch('copy')">复制到…</el-button>
          <span class="hint">硬链接替换时，每组文件链接到各自组的保留文件</span>
        </div>
      </el-card>

      <div class="toolbar">
        <el-select v-model="kindFilter" size="small" style="width: 130px">
          <el-option label="全部类型" value="all" />
          <el-option label="精确重复" value="exact" />
          <el-option label="文件名模糊" value="fuzzy_name" />
          <el-option label="相似图片" value="similar_image" />
          <el-option label="音乐重复" value="music_tag" />
          <el-option label="相似视频" value="similar_video" />
        </el-select>
        <el-button size="small" @click="activeNames = visibleGroups.map(({ i }) => String(i))">全部展开</el-button>
        <el-button size="small" @click="activeNames = []">全部折叠</el-button>
      </div>

      <el-empty v-if="visibleGroups.length === 0" description="当前筛选条件下没有分组" :image-size="60" />

      <el-collapse v-model="activeNames">
        <el-collapse-item v-for="{ g, i } in visibleGroups" :key="g.files[0]?.path + i" :name="String(i)">
          <template #title>
            <div class="group-title">
              <el-checkbox
                :model-value="isAllChecked(g, i)"
                @change="(v: boolean) => toggleAll(g, i, !!v)"
                @click.stop
              />
              <span class="gt-count">{{ g.files.length }} 个文件</span>
              <el-tag size="small" :type="kindInfo(g.kind).type">{{ kindInfo(g.kind).text }}</el-tag>
              <el-tag v-if="g.kind === 'exact'" size="small" type="info">每个 {{ formatBytes(g.file_size) }}</el-tag>
              <el-tag v-else size="small" type="info">大小不等</el-tag>
              <el-tag size="small" type="success">可释放 {{ formatBytes(g.reclaimable) }}</el-tag>
            </div>
          </template>

          <div class="group-body">
            <el-table
              :data="g.files"
              size="small"
              :row-key="(f: FileEntry) => f.path"
              border
              @header-dragend="(newWidth: number, _oldWidth: number, column: any) => onHeaderDragend(column.property, newWidth)"
            >
              <el-table-column width="48" align="center" :resizable="false">
                <template #default="{ row }">
                  <el-checkbox
                    :model-value="checked[i]?.has(row.path) || false"
                    @change="(v: boolean) => onCheck(g, i, row.path, !!v)"
                  />
                </template>
              </el-table-column>
              <el-table-column label="文件路径" prop="path" :width="colWidths.path">
                <template #default="{ row }">
                  <span class="path-cell">{{ row.path }}</span>
                  <!-- 未勾选的文件一律标记「保留」：勾选表示要处理（无标签），未勾选表示将保留 -->
                  <el-tag
                    v-if="!checked[i]?.has(row.path)"
                    size="small"
                    type="warning"
                    style="margin-left: 6px"
                  >保留</el-tag>
                </template>
              </el-table-column>
              <el-table-column label="大小" prop="size" :width="colWidths.size">
                <template #default="{ row }">{{ formatBytes(row.size) }}</template>
              </el-table-column>
              <el-table-column label="修改时间" prop="modified" :width="colWidths.modified">
                <template #default="{ row }">{{ formatDate(row.modified) }}</template>
              </el-table-column>
              <el-table-column label="创建时间" prop="created" :width="colWidths.created">
                <template #default="{ row }">{{ formatDate(row.created) }}</template>
              </el-table-column>
            </el-table>
          </div>
        </el-collapse-item>
      </el-collapse>
    </template>

    <!-- 批量设置保留对话框 -->
    <el-dialog v-model="keepDialog" title="批量设置保留文件" width="92%" style="max-width: 460px">
      <p class="dlg-desc">
        设置保留文件：<b>目录类条件</b>将所有符合条件的文件都设为保留（不勾选）；<b>其他条件</b>每组重排，第一个为保留文件。
      </p>
      <el-select v-model="keepCriterion" style="width: 100%">
        <el-option label="保留文件名最长" value="name_long" />
        <el-option label="保留文件名最短" value="name_short" />
        <el-option label="保留路径名最长" value="path_long" />
        <el-option label="保留路径名最短" value="path_short" />
        <el-option label="保留路径最深（子目录里）" value="depth_deep" />
        <el-option label="保留路径最浅（靠根目录）" value="depth_shallow" />
        <el-option label="保留创建时间最新" value="created_new" />
        <el-option label="保留创建时间最旧" value="created_old" />
        <el-option label="保留修改时间最新" value="modified_new" />
        <el-option label="保留修改时间最旧" value="modified_old" />
        <el-option label="保留位于指定目录及子目录" value="path_under_dir" />
        <el-option label="保留位于指定目录及子目录外" value="path_outside_dir" />
      </el-select>
      <el-form v-if="isKeepDirCriterion()" label-width="50px" style="margin-top: 10px">
        <el-form-item label="目录">
          <el-input v-model="keepDir" placeholder="未选择目录" readonly>
            <template #append>
              <el-button @click="pickKeepDir">选择…</el-button>
            </template>
          </el-input>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="keepDialog = false">取消</el-button>
        <el-button type="primary" @click="applyKeepCriterion">应用</el-button>
      </template>
    </el-dialog>

    <!-- 按条件批量选择对话框 -->
    <el-dialog v-model="selectDialog" title="按条件批量勾选副本" width="92%" style="max-width: 460px">
      <p class="dlg-desc">勾选所有满足条件的文件（含各组保留文件；删除含保留文件时会弹出强警告）。</p>
      <el-form label-width="80px">
        <el-form-item label="条件">
          <el-select v-model="selCriterion" style="width: 100%">
            <el-option label="文件名长度 ≥（字符）" value="name_ge" />
            <el-option label="文件名长度 ≤（字符）" value="name_le" />
            <el-option label="路径名长度 ≥（字符）" value="name_length_ge" />
            <el-option label="路径名长度 ≤（字符）" value="name_length_le" />
            <el-option label="路径深度 ≥（层）" value="depth_ge" />
            <el-option label="路径深度 ≤（层）" value="depth_le" />
            <el-option label="创建时间早于（N 天前）" value="created_before" />
            <el-option label="创建时间晚于（N 天前）" value="created_after" />
            <el-option label="修改时间早于（N 天前）" value="modified_before" />
            <el-option label="修改时间晚于（N 天前）" value="modified_after" />
            <el-option label="文件名包含" value="name_contains" />
            <el-option label="文件名不包含" value="name_not_contains" />
            <el-option label="位于指定目录及子目录" value="path_under_dir" />
            <el-option label="位于指定目录及子目录外" value="path_outside_dir" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="isDirCriterion()" label="目录">
          <el-input v-model="selDir" placeholder="未选择目录" readonly>
            <template #append>
              <el-button @click="pickSelDir">选择…</el-button>
            </template>
          </el-input>
        </el-form-item>
        <el-form-item v-else :label="isTextCriterion() ? '文本' : '数值'">
          <el-input v-if="isTextCriterion()" v-model="selValueText" placeholder="如：副本 / copy" />
          <el-input-number v-else v-model="selValue" :min="0" :max="10000" />
          <span v-if="!isTextCriterion()" class="unit">{{ selUnit() }}</span>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="selectDialog = false">取消</el-button>
        <el-button type="primary" @click="applyBatchSelect">应用选择</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.results-page {
  /* 自适应铺满：窗口缩放时卡片/表格随宽度伸展 */
}
.summary {
  margin-bottom: 10px;
}
.summary-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.spacer {
  flex: 1;
}
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.toolbar-card {
  margin-bottom: 10px;
}
.tool-row {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  padding: 4px 0;
}
.tool-label {
  color: #606266;
  font-size: 13px;
}
.hl {
  color: #f56c6c;
}
.hint {
  color: #909399;
  font-size: 12px;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}
.group-title {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding-right: 12px;
}
.gt-count {
  font-weight: 600;
}
.group-body {
  padding: 4px 8px 12px;
}
.dlg-desc {
  color: #909399;
  font-size: 12px;
  margin: 0 0 10px;
}
.unit {
  margin-left: 8px;
  color: #909399;
  font-size: 12px;
}
</style>
