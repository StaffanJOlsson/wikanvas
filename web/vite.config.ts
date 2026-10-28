// Vite and Vitest configuration for Quire's web client (docs/ARCHITECTURE.md §4.2, §9, §11).
// Run through scripts/dev.sh only: `scripts/dev.sh pnpm --dir web dev|build|test`. For `dev`, publish the port with
// QUIRE_DEV_PORTS=127.0.0.1:5173:5173 and run the server on 127.0.0.1:8080, which the proxy below reaches.
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

const server = 'http://127.0.0.1:8080';

export default defineConfig({
  plugins: [react()],
  server: {
    host: '0.0.0.0',
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': { target: server },
      '/inkboard': { target: server },
    },
  },
  build: {
    outDir: 'dist',
    // §11: `/assets/*` is served immutable, so everything Vite emits there is content-hashed.
    assetsDir: 'assets',
    sourcemap: false,
  },
  test: {
    environment: 'happy-dom',
    setupFiles: ['src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    // The build caps allow 6 CPUs (docs/ARCHITECTURE.md §13.1); tests use at most 4 workers (PLAN W0b).
    maxWorkers: 4,
    restoreMocks: true,
    unstubGlobals: true,
  },
});
