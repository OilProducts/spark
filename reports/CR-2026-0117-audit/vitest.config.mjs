import path from 'node:path'
import { fileURLToPath } from 'node:url'
import original from '../../.spark/checkouts/run-18d45752f4ec6770/frontend/vitest.config.ts'

const root = path.dirname(fileURLToPath(import.meta.url))
const frontend = path.resolve(root, '../../.spark/checkouts/run-18d45752f4ec6770/frontend')
export default {
  ...original,
  root,
  resolve: { alias: {
    ...original.resolve.alias,
    react: `${frontend}/node_modules/react`,
    vitest: `${frontend}/node_modules/vitest/dist/index.js`,
    '@testing-library/react': `${frontend}/node_modules/@testing-library/react/dist/index.js`,
    '@testing-library/user-event': `${frontend}/node_modules/@testing-library/user-event/dist/esm/index.js`,
  } },
  server: { fs: { allow: [root, frontend] } },
  test: {
    ...original.test,
    include: ['pending-navigation.test.tsx'],
    setupFiles: [`${frontend}/src/test/setup.ts`],
  },
}
