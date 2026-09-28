// Fails unless the given .wasm module has zero imports (wasm32-unknown-unknown, no host functions).
// Usage: node scripts/check-wasm-imports.mjs <module.wasm>
import { readFileSync } from 'node:fs';
const path = process.argv[2];
const mod = new WebAssembly.Module(readFileSync(path));
const imports = WebAssembly.Module.imports(mod);
const exports = WebAssembly.Module.exports(mod).map((e) => e.name);
console.log(`${path}: ${imports.length} imports, ${exports.length} exports`);
for (const i of imports) console.log(`  import ${i.module}.${i.name} (${i.kind})`);
process.exit(imports.length === 0 ? 0 : 1);
