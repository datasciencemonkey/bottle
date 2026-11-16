// Copy the two Rive runtimes from node_modules into web/vendor (gitignored).
import { cpSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
for (const rt of ["webgl2", "canvas"]) {
  const out = join(root, "web", "vendor", rt);
  mkdirSync(out, { recursive: true });
  for (const f of ["rive.js", "rive.wasm"]) {
    cpSync(join(root, "node_modules", "@rive-app", rt, f), join(out, f));
  }
}
console.log("vendor copied");
