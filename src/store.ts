// 全局共享状态（供状态栏/各视图跨组件读写）
import { reactive } from 'vue'

export const scanState = reactive({
  scanning: false,
  progressMsg: '',
  progressPct: 0,
})

export const resultState = reactive({
  selectedCount: 0,
  reclaimable: 0,
  groupCount: 0,
})
