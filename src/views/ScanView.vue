<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import {
  runScan,
  cancelScan,
  defaultCachePath,
  onScanProgress,
  pickDirectory,
  type ScanResult,
  type ProgressUpdate,
  type KeepStrategy,
} from '../api'

const emit = defineEmits<{ (e: 'scanned', r: ScanResult): void }>()
defineProps<{ result: ScanResult | null }>()

const paths = ref<string[]>([])
const recursive = ref(true)
const minSizeMB = ref(0)
const maxSizeMB = ref(0)
const onlyExt = ref('')
const excludeExt = ref('')
const excludePaths = ref('')
const followSymlinks = ref(false)
const keepStrategy = ref<KeepStrategy>('keep_oldest')
const useCache = ref(true)
const cachePath = ref('')
const scanning = ref(false)
const progressMsg = ref('')
const progressPct = ref(0)

let unlisten: (() => void) | null = null

onMounted(async () => {
  try {
    cachePath.value = await defaultCachePath()
  } catch {
    /* 非 Tauri 环境忽略 */
  }
  unlisten = await onScanProgress((p: ProgressUpdate) => {
    progressMsg.value = p.message
    let pct = 0
    if (p.phase === 'collect') {
      pct = p.total > 0 ? Math.min(15, Math.round(15 * (p.done / p.total))) : 5
    } else if (p.phase === 'hash') {
      pct = 15 + Math.min(80, Math.round(80 * (p.done / Math.max(1, p.total))))
    }
    progressPct.value = Math.min(99, pct)
  })
})

onBeforeUnmount(() => {
  unlisten?.()
})

async function addPath() {
  const dirs = await pickDirectory(true)
  if (!dirs) return
  const list = Array.isArray(dirs) ? dirs : [dirs]
  for (const d of list) {
    if (!paths.value.includes(d)) paths.value.push(d)
  }
}

function removePath(i: number) {
  paths.value.splice(i, 1)
}

function parseList(s: string): string[] {
  return s
    .split(/[,，;；\s]+/)
    .map((x) => x.trim().toLowerCase())
    .filter(Boolean)
}

async function startScan() {
  if (paths.value.length === 0) {
    ElMessage.warning('请先添加要扫描的目录')
    return
  }
  scanning.value = true
  progressMsg.value = '准备扫描...'
  progressPct.value = 0
  const options = {
    paths: [...paths.value],
    recursive: recursive.value,
    min_size: Math.round(minSizeMB.value * 1024 * 1024),
    max_size: Math.round(maxSizeMB.value * 1024 * 1024),
    only_extensions: parseList(onlyExt.value),
    exclude_extensions: parseList(excludeExt.value),
    exclude_paths: parseList(excludePaths.value),
    follow_symlinks: followSymlinks.value,
    max_depth: null,
    keep_strategy: keepStrategy.value,
    use_cache: useCache.value,
    cache_path: cachePath.value,
  }
  try {
    const result = await runScan(options)
    if (result.groups.length === 0) {
      ElMessage.success(
        `扫描完成：未发现重复文件（${result.scanned_files} 个文件，耗时 ${(result.elapsed_ms / 1000).toFixed(1)}s）`,
      )
    } else {
      ElMessage.success(`发现 ${result.groups.length} 组重复文件，可释放 ${(result.groups.reduce((s, g) => s + g.reclaimable, 0) / 1048576).toFixed(1)} MB`)
      emit('scanned', result)
    }
  } catch (e) {
    ElMessage.error(`扫描失败：${e}`)
  } finally {
    scanning.value = false
    progressPct.value = 0
    progressMsg.value = ''
  }
}

async function stopScan() {
  await cancelScan()
  progressMsg.value = '正在取消...'
}
</script>

<template>
  <div class="page scan-page">
    <el-card shadow="never" class="block">
      <template #header>
        <div class="card-header">
          <span>① 选择扫描目录</span>
          <el-button type="primary" plain size="small" @click="addPath">＋ 添加目录</el-button>
        </div>
      </template>
      <el-empty v-if="paths.length === 0" description="尚未添加目录" :image-size="60" />
      <div v-else class="path-list">
        <div v-for="(p, i) in paths" :key="p + i" class="path-row">
          <el-icon><Folder /></el-icon>
          <span class="path-cell flex-1">{{ p }}</span>
          <el-button link type="danger" size="small" @click="removePath(i)">移除</el-button>
        </div>
      </div>
    </el-card>

    <el-card shadow="never" class="block">
      <template #header><span>② 扫描选项</span></template>
      <el-form label-width="110px" size="default">
        <el-form-item label="递归子目录">
          <el-switch v-model="recursive" />
        </el-form-item>
        <el-form-item label="最小文件大小">
          <el-input-number v-model="minSizeMB" :min="0" :step="1" /> <span class="unit">MB（0 = 不限）</span>
        </el-form-item>
        <el-form-item label="最大文件大小">
          <el-input-number v-model="maxSizeMB" :min="0" :step="1" /> <span class="unit">MB（0 = 不限）</span>
        </el-form-item>
        <el-form-item label="仅扫描扩展名">
          <el-input v-model="onlyExt" placeholder="如：jpg,png,mp4（留空 = 全部）" />
        </el-form-item>
        <el-form-item label="排除扩展名">
          <el-input v-model="excludeExt" placeholder="如：tmp,bak" />
        </el-form-item>
        <el-form-item label="排除路径">
          <el-input v-model="excludePaths" placeholder="路径包含这些文字则跳过，如：node_modules,.git" />
        </el-form-item>
        <el-form-item label="保留策略">
          <el-select v-model="keepStrategy" style="width: 240px">
            <el-option label="保留最旧（默认）" value="keep_oldest" />
            <el-option label="保留最新" value="keep_newest" />
            <el-option label="保留最大" value="keep_largest" />
            <el-option label="保留扫描顺序第一个" value="keep_first" />
          </el-select>
        </el-form-item>
        <el-form-item label="跟随符号链接">
          <el-switch v-model="followSymlinks" />
        </el-form-item>
        <el-form-item label="哈希缓存">
          <el-switch v-model="useCache" />
          <span class="unit">{{ cachePath }}</span>
        </el-form-item>
      </el-form>
    </el-card>

    <div class="action-bar">
      <template v-if="!scanning">
        <el-button type="primary" size="large" @click="startScan">开始扫描</el-button>
      </template>
      <template v-else>
        <el-button type="warning" size="large" @click="stopScan">取消扫描</el-button>
      </template>
      <div v-if="scanning" class="progress-wrap">
        <el-progress :percentage="progressPct" :stroke-width="12" :show-text="true" />
        <div class="progress-msg">{{ progressMsg }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.scan-page {
  max-width: 860px;
  margin: 0 auto;
}
.block {
  margin-bottom: 14px;
}
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.path-list {
  max-height: 180px;
  overflow: auto;
}
.path-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 4px;
  border-bottom: 1px dashed #ebeef5;
}
.flex-1 {
  flex: 1;
}
.unit {
  margin-left: 8px;
  color: #909399;
  font-size: 12px;
  word-break: break-all;
}
.action-bar {
  display: flex;
  align-items: center;
  gap: 20px;
  margin-top: 6px;
}
.progress-wrap {
  flex: 1;
  max-width: 420px;
}
.progress-msg {
  margin-top: 4px;
  color: #909399;
  font-size: 12px;
}
</style>
