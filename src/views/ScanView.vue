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
import { scanState } from '../store'

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
// M2：模糊匹配与相似图片
const fuzzyFilename = ref(false)
const fuzzyThreshold = ref(80)
const fuzzySameDir = ref(true)
const similarImages = ref(false)
const imageThreshold = ref(10)
// M3：音乐去重 + 清理工具
const musicDedup = ref(false)
const musicThreshold = ref(80)
const toolEmptyFolders = ref(false)
const toolBigFiles = ref(false)
const toolBigFilesCount = ref(50)
const toolTempFiles = ref(false)
// M4：相似视频
const similarVideos = ref(false)
const videoThreshold = ref(10)
const progressPct = ref(0)

let unlisten: (() => void) | null = null

onMounted(async () => {
  try {
    cachePath.value = await defaultCachePath()
  } catch {
    /* 非 Tauri 环境忽略 */
  }
  unlisten = await onScanProgress((p: ProgressUpdate) => {
    scanState.progressMsg = p.message
    let pct = 0
    if (p.phase === 'collect') {
      pct = p.total > 0 ? Math.min(15, Math.round(15 * (p.done / p.total))) : 5
    } else if (p.phase === 'hash') {
      pct = 15 + Math.min(60, Math.round(60 * (p.done / Math.max(1, p.total))))
    } else if (p.phase === 'images' || p.phase === 'videos' || p.phase === 'music' || p.phase === 'fuzzy' || p.phase === 'tools') {
      pct = 75 + Math.min(24, Math.round(24 * (p.done / Math.max(1, p.total))))
    }
    progressPct.value = Math.min(99, pct)
    scanState.progressPct = progressPct.value
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
  scanState.scanning = true
  scanState.progressMsg = '准备扫描...'
  progressPct.value = 0
  scanState.progressPct = 0
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
    fuzzy_filename: fuzzyFilename.value,
    fuzzy_threshold: fuzzyThreshold.value,
    fuzzy_same_dir_only: fuzzySameDir.value,
    similar_images: similarImages.value,
    image_threshold: imageThreshold.value,
    music_dedup: musicDedup.value,
    music_threshold: musicThreshold.value,
    tool_empty_folders: toolEmptyFolders.value,
    tool_big_files: toolBigFiles.value,
    tool_big_files_count: toolBigFilesCount.value,
    tool_temp_files: toolTempFiles.value,
    similar_videos: similarVideos.value,
    video_threshold: videoThreshold.value,
    ffmpeg_path: '',
  }
  try {
    const result = await runScan(options)
    if (result.groups.length === 0 && result.tools.length === 0) {
      ElMessage.success(`扫描完成：未发现重复文件（${result.scanned_files} 个文件，耗时 ${(result.elapsed_ms / 1000).toFixed(1)}s）`)
    } else {
      ElMessage.success(
        `发现 ${result.groups.length} 组重复，可释放 ${(result.groups.reduce((s, g) => s + g.reclaimable, 0) / 1048576).toFixed(1)} MB`,
      )
      emit('scanned', result)
    }
  } catch (e) {
    ElMessage.error(`扫描失败：${e}`)
  } finally {
    scanState.scanning = false
    progressPct.value = 0
    scanState.progressPct = 0
    scanState.progressMsg = ''
  }
}

async function stopScan() {
  await cancelScan()
  scanState.progressMsg = '正在取消...'
}
</script>

<template>
  <div class="page scan-page">
    <!-- 扫描目录 -->
    <section class="fd-card">
      <div class="fd-card-header">
        <span class="fd-icon">&#xE8B7;</span>
        <span>扫描目录</span>
        <div class="spacer" />
        <el-button class="fd-add-btn" @click="addPath">＋ 添加目录</el-button>
      </div>
      <el-empty v-if="paths.length === 0" description="尚未添加目录" :image-size="60" />
      <div v-else class="dir-list">
        <div v-for="(p, i) in paths" :key="p + i" class="dir-row">
          <span class="fd-icon dir-icon">&#xE8B7;</span>
          <span class="path-cell flex-1">{{ p }}</span>
          <button class="dir-remove" @click="removePath(i)">移除</button>
        </div>
      </div>
    </section>

    <!-- 扫描选项 -->
    <section class="fd-card">
      <div class="fd-card-header">
        <span class="fd-icon">&#xE713;</span>
        <span>扫描选项</span>
      </div>
      <el-form label-width="130px" size="default">
        <el-form-item label="递归子目录"><el-switch v-model="recursive" /></el-form-item>
        <el-form-item label="最小文件大小">
          <el-input-number v-model="minSizeMB" :min="0" :step="1" /> <span class="unit">MB（0 = 不限）</span>
        </el-form-item>
        <el-form-item label="最大文件大小">
          <el-input-number v-model="maxSizeMB" :min="0" :step="1" /> <span class="unit">MB（0 = 不限）</span>
        </el-form-item>
        <el-form-item label="仅扫描扩展名">
          <el-input v-model="onlyExt" placeholder="如：jpg,png,mp4（留空 = 全部）" />
        </el-form-item>
        <el-form-item label="排除扩展名"><el-input v-model="excludeExt" placeholder="如：tmp,bak" /></el-form-item>
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
        <el-form-item label="跟随符号链接"><el-switch v-model="followSymlinks" /></el-form-item>
        <el-form-item label="哈希缓存">
          <el-switch v-model="useCache" />
          <span class="unit">{{ cachePath }}</span>
        </el-form-item>
      </el-form>
    </section>

    <!-- 智能匹配 -->
    <section class="fd-card">
      <div class="fd-card-header">
        <span class="fd-icon">&#xE8A5;</span>
        <span>智能匹配</span>
        <span class="hint">模糊/相似结果为建议项，执行删除前请人工确认</span>
      </div>
      <el-form label-width="130px" size="default">
        <el-form-item label="文件名模糊匹配">
          <el-switch v-model="fuzzyFilename" />
          <span class="unit">按文件名相似度找重复（dupeGuru 算法）</span>
        </el-form-item>
        <template v-if="fuzzyFilename">
          <el-form-item label="相似度阈值">
            <el-slider v-model="fuzzyThreshold" :min="60" :max="99" :step="1" show-input style="max-width: 260px" />
            <span class="unit">{{ fuzzyThreshold }}%</span>
          </el-form-item>
          <el-form-item label="仅同目录比较">
            <el-switch v-model="fuzzySameDir" />
            <span class="unit">关闭则跨目录比较（可能产生较多结果）</span>
          </el-form-item>
        </template>
        <el-form-item label="相似图片查找">
          <el-switch v-model="similarImages" />
          <span class="unit">感知哈希，识别不同尺寸/格式/压缩率的相似图</span>
        </el-form-item>
        <template v-if="similarImages">
          <el-form-item label="相似度阈值">
            <el-slider v-model="imageThreshold" :min="1" :max="20" :step="1" show-input style="max-width: 260px" />
            <span class="unit">汉明距离 ≤ {{ imageThreshold }} / 63</span>
          </el-form-item>
        </template>
      </el-form>
    </section>

    <!-- 音乐去重 -->
    <section class="fd-card">
      <div class="fd-card-header">
        <span class="fd-icon">&#xE8D6;</span>
        <span>音乐去重</span>
      </div>
      <el-form label-width="130px" size="default">
        <el-form-item label="音乐标签去重">
          <el-switch v-model="musicDedup" />
          <span class="unit">按艺术家+标题标签找重复歌曲</span>
        </el-form-item>
        <template v-if="musicDedup">
          <el-form-item label="相似度阈值">
            <el-slider v-model="musicThreshold" :min="60" :max="99" :step="1" show-input style="max-width: 260px" />
            <span class="unit">{{ musicThreshold }}%</span>
          </el-form-item>
        </template>
      </el-form>
    </section>

    <!-- 清理工具 -->
    <section class="fd-card">
      <div class="fd-card-header">
        <span class="fd-icon">&#xE74D;</span>
        <span>清理工具</span>
      </div>
      <el-form label-width="130px" size="default">
        <el-form-item label="空文件夹">
          <el-switch v-model="toolEmptyFolders" />
          <span class="unit">查找只含空子目录的目录（忽略 desktop.ini）</span>
        </el-form-item>
        <el-form-item label="大文件">
          <el-switch v-model="toolBigFiles" />
          <span class="unit">列出最大的</span>
          <el-input-number v-model="toolBigFilesCount" :min="1" :max="500" size="small" style="width: 90px; margin-left: 6px" />
          <span class="unit">个文件</span>
        </el-form-item>
        <el-form-item label="临时文件">
          <el-switch v-model="toolTempFiles" />
          <span class="unit">*.tmp / ~$ 锁定文件 / 临时目录中的文件</span>
        </el-form-item>
      </el-form>
    </section>

    <!-- 相似视频 -->
    <section class="fd-card">
      <div class="fd-card-header">
        <span class="fd-icon">&#xE714;</span>
        <span>相似视频</span>
        <span class="hint">需要 ffmpeg.exe（应用目录或 PATH）</span>
      </div>
      <el-form label-width="130px" size="default">
        <el-form-item label="相似视频查找">
          <el-switch v-model="similarVideos" />
          <span class="unit">FFmpeg 抽帧 + 帧感知哈希，识别不同分辨率/码率的视频</span>
        </el-form-item>
        <template v-if="similarVideos">
          <el-form-item label="相似度阈值">
            <el-slider v-model="videoThreshold" :min="1" :max="20" :step="1" show-input style="max-width: 260px" />
            <span class="unit">帧汉明距离 ≤ {{ videoThreshold }} / 63</span>
          </el-form-item>
        </template>
      </el-form>
    </section>

    <div class="action-bar">
      <template v-if="!scanState.scanning">
        <el-button type="primary" size="large" @click="startScan">开始扫描</el-button>
      </template>
      <template v-else>
        <el-button type="warning" size="large" @click="stopScan">取消扫描</el-button>
      </template>
      <div v-if="scanState.scanning" class="progress-wrap">
        <el-progress :percentage="progressPct" :stroke-width="10" :show-text="false" />
        <div class="progress-msg">{{ scanState.progressMsg }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.scan-page {
  /* 自适应铺满：窗口缩放时卡片随宽度伸展 */
}
.spacer {
  flex: 1;
}
.fd-add-btn {
  color: var(--fd-accent) !important;
  font-weight: 600;
}
.unit {
  margin-left: 8px;
  color: var(--fd-text-2);
  font-size: 12px;
  word-break: break-all;
}
.hint {
  color: var(--fd-text-2);
  font-size: 12px;
  font-weight: 400;
}
.dir-list {
  max-height: 180px;
  overflow: auto;
}
.dir-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 4px;
  border-bottom: 1px dashed var(--fd-border);
}
.dir-icon {
  color: var(--fd-text-2);
}
.flex-1 {
  flex: 1;
}
.dir-remove {
  border: none;
  background: none;
  color: var(--fd-text-2);
  font-size: 13px;
  font-family: var(--fd-font);
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 6px;
  transition: color 0.12s ease, background 0.12s ease;
}
.dir-remove:hover {
  color: var(--fd-accent);
  background: var(--fd-surface-2);
}
.action-bar {
  display: flex;
  align-items: center;
  gap: 20px;
  margin-top: 4px;
  padding-bottom: 12px;
}
.progress-wrap {
  flex: 1;
  max-width: 420px;
}
.progress-msg {
  margin-top: 4px;
  color: var(--fd-text-2);
  font-size: 12px;
}
</style>
