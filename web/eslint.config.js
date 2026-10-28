import js from '@eslint/js'
import svelte from 'eslint-plugin-svelte'
import globals from 'globals'
import ts from 'typescript-eslint'

export default ts.config(
  { ignores: ['dist/', '.svelte-kit/', 'src/lib/generated/', 'src/lib/engine/wasm/'] },
  js.configs.recommended,
  ...ts.configs.recommended,
  ...svelte.configs.recommended,
  {
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
      // Typed rules read the types of the project (tsconfig.json). A TypeScript file outside it is linted from the default project.
      parserOptions: {
        projectService: { allowDefaultProject: ['playwright.bench.config.ts'] },
        extraFileExtensions: ['.svelte'],
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
  {
    files: ['**/*.svelte', '**/*.svelte.ts'],
    languageOptions: { parserOptions: { parser: ts.parser } },
  },
  {
    // The typed rules that catch bugs (#379): a promise nobody awaits or whose rejection is lost, an async function handed to a callback
    // that ignores it, a method detached from its object, a value that prints as "[object Object]".
    // Left out on purpose: the `no-unsafe-*` family (it flags every `any`, mostly the results of page.evaluate in the browser tests),
    // `require-await` and `no-unnecessary-type-assertion` (style, hundreds of hits, no bug found among them).
    files: ['**/*.ts', '**/*.svelte'],
    rules: {
      '@typescript-eslint/await-thenable': 'error',
      '@typescript-eslint/no-base-to-string': 'error',
      '@typescript-eslint/no-duplicate-type-constituents': 'error',
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-implied-eval': 'error',
      '@typescript-eslint/no-misused-promises': 'error',
      '@typescript-eslint/no-redundant-type-constituents': 'error',
      '@typescript-eslint/prefer-promise-reject-errors': 'error',
      '@typescript-eslint/restrict-template-expressions': 'error',
      '@typescript-eslint/unbound-method': 'error',
    },
  },
  {
    // `expect(object.method)` is how a mock is asserted, and the tests run generated scripts on purpose (`new Function`).
    files: ['**/*.test.ts', 'tests/**'],
    rules: { '@typescript-eslint/unbound-method': 'off', '@typescript-eslint/no-implied-eval': 'off' },
  },
  // Plain JavaScript files (the config itself, scripts) have no types to read.
  { files: ['**/*.js', '**/*.mjs'], ...ts.configs.disableTypeChecked },
)
