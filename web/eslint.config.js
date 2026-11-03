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
  {
    // The schema bundle (42 MB at #436, 1.4 MB since #559): TypeScript infers the type of a JSON import, and svelte-check runs out of memory on it.
    // Read it at run time instead, or use the generated types. `src/lib/generated/` is ignored above, so it stays out of reach.
    // The whole web app is guarded, not only `src/`: svelte-check types the tests and scripts too (`tsconfig.json`), and a test hit the OOM (#436).
    files: ['**/*.{ts,svelte,js,mjs,cjs,mts,cts}'],
    rules: {
      'no-restricted-syntax': [
        'error',
        ...[
          'ImportDeclaration[source.value=PATTERN]',
          'ExportNamedDeclaration[source.value=PATTERN]',
          'ExportAllDeclaration[source.value=PATTERN]',
          'ImportExpression[source.value=PATTERN]',
          // `typeof import('…')` in a type position and `import x = require('…')`: both make TypeScript infer the JSON type.
          'TSImportType Literal[value=PATTERN]',
          'TSExternalModuleReference > Literal[value=PATTERN]',
          'CallExpression[callee.name="require"] > Literal[value=PATTERN]',
        ].map((selector) => ({
          selector: selector.replace('PATTERN', '/generated.schema\\.json$/'),
          message:
            'Do not import generated/schema.json: TypeScript infers the type of its large content (42 MB when svelte-check ran out of memory, #436). Read it at run time (readFileSync + JSON.parse) or use the generated types.',
        })),
      ],
    },
  },
  // Plain JavaScript files (the config itself, scripts) have no types to read.
  { files: ['**/*.js', '**/*.mjs'], ...ts.configs.disableTypeChecked },
)
