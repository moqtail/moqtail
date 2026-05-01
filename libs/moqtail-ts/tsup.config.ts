import { defineConfig } from 'tsup'

export default defineConfig({
  entry: {
    index: 'src/index.ts',
    client: 'src/client/index.ts',
    model: 'src/model/index.ts',
    util: 'src/util/index.ts',
  },
  format: ['cjs', 'esm'],
  dts: {
    compilerOptions: {
      ignoreDeprecations: '6.0',
    },
  },
  outDir: 'dist',
  splitting: false,
  sourcemap: false,
  clean: true,
  treeshake: true,
  define: {
    'import.meta.vitest': 'undefined',
  },
})
