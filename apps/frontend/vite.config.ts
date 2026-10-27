import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

const backendUrl = process.env.VITE_BACKEND_URL;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  build: {
    cssTarget: ['chrome88', 'safari14', 'firefox78', 'edge88'],
  },
  server: {
    host: '127.0.0.1',
    proxy: backendUrl
      ? Object.fromEntries(
          ['/api', '/auth/github', '/webhooks', '/records', '/runs'].map((path) => [
            path,
            { target: backendUrl, changeOrigin: true, ws: path === '/runs' },
          ]),
        )
      : undefined,
  },
});
