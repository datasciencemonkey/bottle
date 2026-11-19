/// <reference types="vitest/config" />
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig, type Plugin } from 'vite';

const root = import.meta.dirname;

/** Where `rive mog-rive --publish` writes the mascot (gitignored; never commit a `.riv`). */
export const MOG_RIV_BUILD = resolve(root, 'mog-rive', 'build', 'mog-rive.riv');
/** Where the MOG view loads it: `/mascot/mog.riv` on the app origin (ADR 0027). */
export const MOG_RIV_ASSET = 'mascot/mog.riv';

/**
 * Ship the mascot `.riv` as a bundled file (ADR 0027, SEC-20a). The file is built, not
 * committed, so a build without it still works and the MOG view shows the PNG fallback (UX-09b).
 */
export function mogRivAsset(file: string = MOG_RIV_BUILD): Plugin {
  return {
    name: 'bottle-mog-riv',
    configureServer(server) {
      server.middlewares.use(`/${MOG_RIV_ASSET}`, (_req, res, next) => {
        if (!existsSync(file)) return next();
        res.setHeader('Content-Type', 'application/octet-stream');
        res.end(readFileSync(file));
      });
    },
    generateBundle() {
      if (!existsSync(file)) {
        this.warn(`${file} is missing: the MOG view will show the PNG fallback (ADR 0027)`);
        return;
      }
      this.emitFile({ type: 'asset', fileName: MOG_RIV_ASSET, source: readFileSync(file) });
    },
  };
}

// One HTML entry per window kind (ARC-3). Tauri expects a fixed dev port.
export default defineConfig({
  // tailwindcss() only transforms CSS that actually uses Tailwind's `@import`/`@theme` syntax
  // (ui-1, ADR 0104): src/views/hub/tailwind.css. mog.css and card.css have none, so they pass
  // through untouched; mog.html and card.html never load tailwind.css at all.
  plugins: [react(), tailwindcss(), mogRivAsset()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: '127.0.0.1' },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true,
    // The CSP has no `data:` source, so no asset may be inlined as a data URL (ADR 0027).
    assetsInlineLimit: 0,
    rollupOptions: {
      input: {
        brain: resolve(root, 'brain.html'),
        hub: resolve(root, 'hub.html'),
        mog: resolve(root, 'mog.html'),
        card: resolve(root, 'card.html'),
      },
    },
  },
  test: {
    globals: true,
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}', 'tests/**/*.test.ts'],
    coverage: {
      provider: 'v8',
      reportsDirectory: 'reports/coverage-ts',
      reporter: ['text-summary', 'lcov', 'json-summary'],
      include: ['src/**/*.{ts,tsx}'],
    },
  },
});
