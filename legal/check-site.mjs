// Generated copies let each website build/deploy as an independent repository.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(fs.readFileSync(path.join(root, "public/legal/build-manifest.json"), "utf8"));
for (const [file, expected] of Object.entries(manifest.files)) {
  const actual = createHash("sha256").update(fs.readFileSync(path.join(root, file))).digest("hex");
  if (actual !== expected) throw new Error(`Stale legal artifact: ${file}. Regenerate legal documents and notices before publishing.`);
}
console.log(`Site legal artifacts verified (${manifest.updated}; ${Object.keys(manifest.files).length} files).`);
