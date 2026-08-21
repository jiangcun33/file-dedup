<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import ScanView from './views/ScanView.vue'
import ResultsView from './views/ResultsView.vue'
import type { ScanResult } from './api'

const view = ref<'scan' | 'results'>('scan')
const result = ref<ScanResult | null>(null)
const groupsCount = ref(0)
const appVersion = ref('')

onMounted(async () => {
  try {
    appVersion.value = await invoke<string>('app_version')
    const full = `文件去重 v${appVersion.value}`
    // 同时设置文档标题与原生窗口标题（避免 WebView2 标题同步覆盖）
    document.title = full
    await getCurrentWindow().setTitle(full)
  } catch {
    /* 非 Tauri 环境忽略 */
  }
})

function onScanned(r: ScanResult) {
  result.value = r
  groupsCount.value = r.groups.length
  view.value = 'results'
}
</script>

<template>
  <div class="app-shell">
    <header class="app-header">
      <span class="logo">🗂️</span>
      <span class="title">文件去重{{ appVersion ? ` v${appVersion}` : '' }}</span>
      <nav class="nav">
        <el-button :type="view === 'scan' ? 'primary' : ''" text @click="view = 'scan'">扫描</el-button>
        <el-button
          :type="view === 'results' ? 'primary' : ''"
          text
          :disabled="!result"
          @click="view = 'results'"
        >
          结果{{ result ? `（${groupsCount} 组）` : '' }}
        </el-button>
      </nav>
    </header>
    <main class="app-main">
      <!-- v-show 保持两个视图常驻，切换回扫描页时保留已选目录与选项 -->
      <ScanView v-show="view === 'scan'" :result="result" @scanned="onScanned" />
      <ResultsView v-if="result" v-show="view === 'results'" :result="result" />
    </main>
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.app-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 16px;
  background: #fff;
  border-bottom: 1px solid #e4e7ed;
  flex: none;
}
.logo {
  font-size: 22px;
}
.title {
  font-size: 17px;
  font-weight: 600;
  margin-right: 8px;
}
.nav {
  display: flex;
  gap: 4px;
}
.app-main {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
/* 子视图填满主区域并在内部滚动，窗口缩放时自适应 */
.app-main > * {
  flex: 1 1 auto;
  min-height: 0;
  min-width: 0;
}
</style>
