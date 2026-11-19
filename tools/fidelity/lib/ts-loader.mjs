// Module hooks that let node import the design's TypeScript as it is: extensionless relative
// imports (`./gl/scene`) resolve to `.ts`, and `.ts` sources are transformed with node's own
// `stripTypeScriptTypes` in transform mode (node >= 22.13), which also handles parameter
// properties (`constructor(private scene: Scene)`), which plain type stripping refuses.
// Register with `module.register('./lib/ts-loader.mjs', import.meta.url)`; run node with
// `--disable-warning=ExperimentalWarning` to silence the transform's warning.
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { fileURLToPath } from 'node:url';

export async function resolve(specifier, context, next) {
  const fromTs = context.parentURL?.endsWith('.ts');
  if (fromTs && specifier.startsWith('.') && !/\.\w+$/.test(specifier)) {
    return next(`${specifier}.ts`, context);
  }
  return next(specifier, context);
}

export async function load(url, context, next) {
  if (!url.endsWith('.ts')) return next(url, context);
  const src = await readFile(fileURLToPath(url), 'utf8');
  return { format: 'module', source: stripTypeScriptTypes(src, { mode: 'transform' }), shortCircuit: true };
}
