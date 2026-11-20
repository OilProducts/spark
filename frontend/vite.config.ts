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
  build: {
    // ponytail: a budget just above today's startup bundle (~1,530 kB), so growth past it still warns.
    // The editor, runs, triggers and missions screens stay mounted while hidden, so they load at startup;
    // loading them on first visit is what would bring this under Vite's 500 kB default.
    chunkSizeWarningLimit: 1600,
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    proxy: {
      '/attractor': backendUrl,
      '/workspace': backendUrl,
      '/attractor/ws': {
        target: wsUrl,
        ws: true,
      },
    }
  }
})
