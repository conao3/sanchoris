import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

const backendUrl = process.env.VITE_BACKEND_URL;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    host: '127.0.0.1',
    proxy: backendUrl
      ? {
          '/api': {
            target: backendUrl,
            changeOrigin: true,
          },
        }
      : undefined,
  },
});
