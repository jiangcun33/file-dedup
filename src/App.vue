<script setup lang="ts">
import { ref } from 'vue'
import ScanView from './views/ScanView.vue'
import ResultsView from './views/ResultsView.vue'
import type { ScanResult } from './api'

const view = ref<'scan' | 'results'>('scan')
const result = ref<ScanResult | null>(null)
const groupsCount = ref(0)

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
      <span class="title">文件去重</span>
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
      <ScanView v-if="view === 'scan'" :result="result" @scanned="onScanned" />
      <ResultsView v-else-if="view === 'results' && result" :result="result" />
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
}
</style>
