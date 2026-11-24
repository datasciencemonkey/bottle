import js from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import globals from 'globals';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  {
    ignores: [
      'dist/**',
      'reports/**',
      'target/**',
      'src-tauri/target/**',
      'src-tauri/gen/**',
      'mog-rive/**',
      'research/**',
      'tools/**',
      'node_modules/**',
      // Throwaway spike code: each spikes/sp-nn/ folder is standalone with its own tooling (docs/spikes/).
      'spikes/**',
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.strict,
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: { globals: { ...globals.browser } },
    plugins: { 'react-hooks': reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      // Stub methods keep the interface parameter names with a leading underscore (#226).
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
    },
  },
  {
    files: ['tests/**/*.ts', 'vite.config.ts', 'eslint.config.js'],
    languageOptions: { globals: { ...globals.node } },
  },
  {
    // The static landing page script (site/, published by .github/workflows/pages.yml).
    files: ['site/**/*.js'],
    languageOptions: { sourceType: 'script', globals: { ...globals.browser } },
  },
);
