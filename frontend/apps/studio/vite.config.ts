import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// The Studio is served by the control plane under /studio/ (D143), so every asset URL starts there.
export default defineConfig({
  base: '/studio/',
  plugins: [react()],
  build: {
    outDir: 'dist',
    sourcemap: false,
    // Monaco is large; the editor is loaded on demand, so the warning is about a chunk that is.
    chunkSizeWarningLimit: 6000,
    rollupOptions: {
      output: { entryFileNames: 'assets/[name].[hash].js', chunkFileNames: 'assets/[name].[hash].js', assetFileNames: 'assets/[name].[hash][extname]' },
    },
  },
  server: {
    port: 5173,
    // In development the API is the control plane on its own port; the browser sees one origin.
    proxy: { '/v1': { target: process.env['SAGAT_API_URL'] ?? 'http://127.0.0.1:8080', changeOrigin: false } },
  },
});
