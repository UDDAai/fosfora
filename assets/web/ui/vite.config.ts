import path from 'node:path'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'
import { viteSingleFile } from 'vite-plugin-singlefile'

// The dev server proxies to a running engine; FOSFORA_ENGINE points it at another one.
const engine = process.env.FOSFORA_ENGINE ?? 'http://127.0.0.1:9002'

// Fosfora serves one HTML document and redirects every other path to /.
// The production build inlines JS and CSS into that document.
export default defineConfig({
  plugins: [react(), tailwindcss(), viteSingleFile()],
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, './src'),
    },
  },
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: {
      '/ws': { target: engine, ws: true },
      '/api': { target: engine },
      '/health': { target: engine },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
})
