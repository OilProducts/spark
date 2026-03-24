import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import tseslint from 'typescript-eslint'
import { defineConfig, globalIgnores } from 'eslint/config'

export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommended,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
  },
  {
    files: ['src/features/**/components/**/*.{ts,tsx}'],
    rules: {
      'no-restricted-imports': ['error', {
        paths: [
          {
            name: '@/lib/attractorClient',
            message: 'Feature presentation components must not call API clients directly. Move API usage into hooks or model loaders.',
            allowTypeImports: true,
          },
          {
            name: '@/lib/workspaceClient',
            message: 'Feature presentation components must not call API clients directly. Move API usage into hooks or model loaders.',
            allowTypeImports: true,
          },
        ],
        patterns: [
          {
            group: ['@/lib/api/*'],
            message: 'Feature presentation components must not import API client modules directly.',
          },
          {
            group: ['@/components/ui/*'],
            message: 'Use the canonical shared UI layer from @/ui/*.',
          },
        ],
      }],
    },
  },
])
