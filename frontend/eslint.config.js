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
    rules: {
      'react-hooks/set-state-in-effect': 'off',
      'react-refresh/only-export-components': 'off',
      '@typescript-eslint/no-unused-vars': ['error', {
        argsIgnorePattern: '^_',
        varsIgnorePattern: '^_',
        caughtErrorsIgnorePattern: '^_',
        destructuredArrayIgnorePattern: '^_',
      }],
    },
  },
  {
    files: ['src/features/**/*.{ts,tsx}', 'src/app/**/*.{ts,tsx}', 'src/App.tsx'],
    ignores: [
      'src/features/**/__tests__/**',
      'src/features/**/*.test.{ts,tsx}',
      'src/features/**/*.spec.{ts,tsx}',
      'src/features/**/hooks/**',
      'src/features/**/model/**',
      'src/features/**/services/**',
      'src/app/use*.{ts,tsx}',
    ],
    rules: {
      '@typescript-eslint/no-restricted-imports': ['error', {
        paths: [
          {
            name: '@/lib/attractorClient',
            message: 'Presentation files must not call API clients directly. Move API usage into feature hooks, services, or model loaders.',
            allowTypeImports: true,
          },
          {
            name: '@/lib/workspaceClient',
            message: 'Presentation files must not call API clients directly. Move API usage into feature hooks, services, or model loaders.',
            allowTypeImports: true,
          },
          {
            name: '@/lib/apiClient',
            message: 'Presentation files must not call API clients directly. Move API usage into feature hooks, services, or model loaders.',
            allowTypeImports: true,
          },
          {
            name: '@/lib/validatedApiClient',
            message: 'Presentation files must not call API clients directly. Move API usage into feature hooks, services, or model loaders.',
            allowTypeImports: true,
          },
        ],
        patterns: [
          {
            // @/lib/api/shared holds plain utilities (isAbortError), not client calls.
            group: ['@/lib/api/*', '!@/lib/api/shared'],
            message: 'Presentation files must not import API client modules directly.',
            allowTypeImports: true,
          },
          {
            // Negated group patterns can't re-include children of an excluded
            // parent dir (gitignore semantics), so allow ui/app via regex instead.
            regex: '^@/components(?!/(ui|app)/)',
            message: 'Shared imports must come from @/components/ui/* or @/components/app/*, or stay inside the owning feature.',
          },
        ],
      }],
    },
  },
  {
    files: ['src/features/**/*.{tsx}', 'src/app/**/*.{tsx}', 'src/App.tsx'],
    ignores: [
      'src/features/**/__tests__/**',
      'src/features/**/*.test.{tsx}',
      'src/features/**/*.spec.{tsx}',
      'src/features/**/hooks/**',
      'src/features/**/model/**',
      'src/features/**/services/**',
      'src/features/workflow-canvas/**',
    ],
    rules: {
      'no-restricted-syntax': ['error',
        {
          selector: "JSXOpeningElement[name.name='button']",
          message: 'Use Button from @/components/ui/button for non-canvas feature controls.',
        },
        {
          selector: "JSXOpeningElement[name.name='input']",
          message: 'Use Input from @/components/ui/input for non-canvas feature controls.',
        },
        {
          selector: "JSXOpeningElement[name.name='select']",
          message: 'Use NativeSelect from @/components/ui/native-select or Select from @/components/ui/select for non-canvas feature controls.',
        },
        {
          selector: "JSXOpeningElement[name.name='textarea']",
          message: 'Use Textarea from @/components/ui/textarea for non-canvas feature controls.',
        },
        {
          selector: "JSXOpeningElement[name.name='label']",
          message: 'Use Label from @/components/ui/label or FieldRow from @/components/app/field-row for non-canvas feature forms.',
        },
      ],
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    ignores: [
      'src/**/__tests__/**',
      'src/**/*.test.{ts,tsx}',
      'src/**/*.spec.{ts,tsx}',
      'src/components/app/dialog-controller.tsx',
    ],
    rules: {
      'no-restricted-globals': ['error',
        {
          name: 'alert',
          message: 'Use useDialogController from @/components/app/dialog-controller for non-canvas alerts.',
        },
        {
          name: 'confirm',
          message: 'Use useDialogController from @/components/app/dialog-controller for non-canvas confirmations.',
        },
        {
          name: 'prompt',
          message: 'Use useDialogController from @/components/app/dialog-controller for non-canvas prompts.',
        },
      ],
      'no-restricted-properties': ['error',
        {
          object: 'window',
          property: 'alert',
          message: 'Use useDialogController from @/components/app/dialog-controller for non-canvas alerts.',
        },
        {
          object: 'window',
          property: 'confirm',
          message: 'Use useDialogController from @/components/app/dialog-controller for non-canvas confirmations.',
        },
        {
          object: 'window',
          property: 'prompt',
          message: 'Use useDialogController from @/components/app/dialog-controller for non-canvas prompts.',
        },
      ],
    },
  },
])
