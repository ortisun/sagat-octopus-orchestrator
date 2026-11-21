import js from '@eslint/js';
import jsxA11y from 'eslint-plugin-jsx-a11y';
import reactHooks from 'eslint-plugin-react-hooks';
import globals from 'globals';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  { ignores: ['dist', 'node_modules', 'src/api/schema.d.ts', 'eslint.config.js', 'scripts/**'] },
  js.configs.recommended,
  ...tseslint.configs.recommendedTypeChecked,
  jsxA11y.flatConfigs.recommended,
  {
    languageOptions: {
      globals: { ...globals.browser },
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
    plugins: { 'react-hooks': reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/consistent-type-imports': 'error',
      // Untrusted text reaches the page only as text: React escapes it, and nothing here may turn
      // it into markup or code.
      'no-restricted-syntax': [
        'error',
        { selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']", message: 'Never render server data as HTML.' },
      ],
      'no-eval': 'error',
      'no-implied-eval': 'error',
    },
  },
  {
    // Architecture decision 0101: the Astryx design system is reached only through the Studio's interface module, and
    // a screen composes that module's components instead of hand-writing a control.
    files: ['src/**/*.{ts,tsx}'],
    ignores: ['src/ui/**'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            { group: ['@astryxdesign/*', '@stylexjs/*'], message: 'Import the Studio interface module (src/ui) instead: only src/ui may depend on Astryx (Architecture decision 0101).' },
          ],
        },
      ],
    },
  },
  {
    files: ['src/**/*.tsx'],
    ignores: ['src/ui/**', 'src/**/*.test.tsx'],
    rules: {
      'no-restricted-syntax': [
        'error',
        { selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']", message: 'Never render server data as HTML.' },
        {
          selector: 'JSXOpeningElement[name.name=/^(button|input|select|textarea|dialog|table|h[1-6])$/]',
          message: 'Use the component of the Studio interface module (src/ui) instead of a raw control, table or heading (Architecture decision 0101). A surface a library owns may disable this rule on its line, with the reason.',
        },
      ],
    },
  },
  {
    files: ['**/*.{js,mjs}', 'vite.config.ts', 'vitest.config.ts'],
    languageOptions: { globals: { ...globals.node } },
  },
);
