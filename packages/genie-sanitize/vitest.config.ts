// Own config so vitest does not load the root vite.config.ts (React plugin, app paths).
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['test/**/*.test.ts'],
    environment: 'node',
  },
});
