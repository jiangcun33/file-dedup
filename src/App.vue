<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import ScanView from './views/ScanView.vue'
import ResultsView from './views/ResultsView.vue'
import { scanState, resultState } from './store'
import type { ScanResult } from './api'

const view = ref<'scan' | 'results'>('scan')
const result = ref<ScanResult | null>(null)
const appVersion = ref('')
const isMaximized = ref(false)

const win = getCurrentWindow()

onMounted(async () => {
  try {
    // 版本号 + 窗口标题
    appVersion.value = await invoke<string>('app_version')
    const full = `文件去重 v${appVersion.value}`
    document.title = full
    await win.setTitle(full)
    // 系统强调色注入 CSS 变量
    const accent = await invoke<string>('get_accent_color')
    document.documentElement.style.setProperty('--fd-accent', accent)
    // 最大化状态监听（用于切换恢复按钮图标）
    isMaximized.value = await win.isMaximized()
    await win.onResized(() => {
      win.isMaximized().then((m) => (isMaximized.value = m))
    })
  } catch {
    /* 非 Tauri 环境忽略 */
  }
})

function switchView(v: 'scan' | 'results') {
  view.value = v
}

function onScanned(r: ScanResult) {
  result.value = r
  resultState.groupCount = r.groups.length
  view.value = 'results'
}

// 每次开始扫描前，清除上一次的扫描结果
function onScanStart() {
  result.value = null
  resultState.groupCount = 0
  resultState.selectedCount = 0
  resultState.reclaimable = 0
}

async function winMinimize() {
  await win.minimize()
}
async function winToggleMaximize() {
  await win.toggleMaximize()
}
async function winClose() {
  await win.close()
}
</script>

<template>
  <div class="app-shell fd-mica">
    <!-- 自定义标题栏（Fluent） -->
    <div class="fd-titlebar" data-tauri-drag-region>
      <span class="fd-title">🗂️ 文件去重</span>
      <span class="fd-title-ver">{{ appVersion ? `v${appVersion}` : '' }}</span>
      <div class="fd-winctrl">
        <button title="最小化" @click="winMinimize"><span class="fd-icon">&#xE921;</span></button>
        <button :title="isMaximized ? '还原' : '最大化'" @click="winToggleMaximize">
          <span class="fd-icon">{{ isMaximized ? '&#xE923;' : '&#xE922;' }}</span>
        </button>
        <button class="fd-close" title="关闭" @click="winClose"><span class="fd-icon">&#xE8BB;</span></button>
      </div>
    </div>

    <!-- 下划线 Tab -->
    <nav class="fd-tabs" style="padding: 0 16px; background: var(--fd-surface); border-bottom: 1px solid var(--fd-border)">
      <button class="fd-tab" :class="{ active: view === 'scan' }" @click="switchView('scan')">扫描</button>
      <button
        class="fd-tab"
        :class="{ active: view === 'results' }"
        :disabled="!result"
        style="opacity: 0.6"
        @click="switchView('results')"
      >
        结果{{ result ? `（${resultState.groupCount}）` : '' }}
      </button>
    </nav>

    <main class="app-main">
      <!-- v-show 保持视图常驻，切换回扫描页保留已选目录与选项 -->
      <ScanView
        v-show="view === 'scan'"
        :result="result"
        @scanned="onScanned"
        @scan-start="onScanStart"
      />
      <ResultsView v-if="result" v-show="view === 'results'" :result="result" @back="switchView('scan')" />
    </main>

    <!-- 底部状态栏（含扫描进度条） -->
    <footer class="fd-statusbar">
      <span class="fd-sb-item">
        <span class="fd-icon">&#xE7F4;</span>
        <template v-if="scanState.scanning">{{ scanState.progressMsg }}</template>
        <template v-else-if="view === 'results'">扫描完成</template>
        <template v-else>就绪</template>
      </span>
      <el-progress
        v-if="scanState.scanning"
        :percentage="scanState.progressPct"
        :stroke-width="6"
        :show-text="false"
        class="sb-progress"
      />
      <span v-if="scanState.scanning" class="fd-sb-item">{{ scanState.progressPct }}%</span>
      <span class="spacer" />
      <span class="fd-sb-item">
        <span class="fd-icon">&#xE8F1;</span>
        <span>已选 <b class="fd-num">{{ resultState.selectedCount }}</b> 个文件</span>
      </span>
      <span class="fd-sb-item">
        <span class="fd-icon">&#xEA18;</span>
        <span>可释放 {{ resultState.reclaimable > 0 ? (resultState.reclaimable / 1048576).toFixed(1) + ' MB' : '-' }}</span>
      </span>
    </footer>
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.app-main {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.app-main > * {
  flex: 1 1 auto;
  min-height: 0;
  min-width: 0;
}
.fd-num {
  color: var(--fd-text);
}
.spacer {
  flex: 1;
}
.sb-progress {
  flex: 1;
  max-width: 320px;
  margin: 0 12px;
}
</style>
