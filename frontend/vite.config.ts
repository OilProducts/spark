import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import path from "path"

const backendUrl = process.env.VITE_BACKEND_URL || 'http://127.0.0.1:8000'
const wsUrl = backendUrl.startsWith('https')
  ? backendUrl.replace(/^https/, 'wss')
  : backendUrl.replace(/^http/, 'ws')

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    proxy: {
      '/api': backendUrl,
      '/run': backendUrl,
      '/preview': backendUrl,
      '/ws': {
        target: wsUrl,
        ws: true,
      },
    }
  }
})
