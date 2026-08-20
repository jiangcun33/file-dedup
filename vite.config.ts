import { defineConfig } from 'vite'
import { fileURLToPath, URL } from 'node:url'
import vue from '@vitejs/plugin-vue'

// 注意：项目可能通过目录联接（junction）访问。将 root 固定为配置文件
// 的真实路径（Node 加载模块时已解析 realpath），避免联接路径与真实路径
// 混用导致构建/监听异常。
const projectRoot = fileURLToPath(new URL('.', import.meta.url))

export default defineConfig({
  root: projectRoot,
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    // 项目可能通过目录联接（junction）访问，真实路径与 serve root 不一致，
    // 关闭 vite 的文件系统严格限制以正常加载源码
    fs: { strict: false },
    // 不监听本机工具链/调研资料等目录，避免文件写入触发 EBUSY
    watch: {
      ignored: ['**/.toolchain/**', '**/research/**', '**/发布/**', '**/.cargo/**', '**/.git/**'],
    },
  },
  // 依赖预构建缓存放到纯 ASCII 路径，规避 junction/realpath 混合导致的
  // 优化器元数据损坏（Cannot read properties of undefined (reading 'imports')）
  cacheDir: 'D:/filededup-vite-cache',
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: 'chrome105',
    minify: 'esbuild',
    sourcemap: false,
    outDir: 'dist',
    emptyOutDir: true,
  },
})
