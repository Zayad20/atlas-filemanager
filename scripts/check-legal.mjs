// Portable release check: web builds need Node, not Python/Cargo installations.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = file => fs.readFileSync(path.join(root, file));
const json = file => JSON.parse(read(file).toString("utf8"));
const digest = value => createHash("sha256").update(value).digest("hex");
const verify = (file, hash) => {
  if (digest(read(file)) !== hash) throw new Error(`Legal input/output changed: ${file}. Regenerate the legal artifacts.`);
};
const generated = json("legal/generated-files.json");
for (const [file, hash] of Object.entries({...generated.sources, ...generated.outputs})) verify(file, hash);
const inventory = json("legal/dependency-inventory.json");
for (const [file, hash] of Object.entries(inventory.inputHashes)) verify(file, hash);
for (const override of Object.values(json("legal/notice-overrides.json"))) {
  for (const notice of override.notices ?? []) if (notice.sha256) verify(notice.path, notice.sha256);
}
for (const asset of json("legal/assets.json")) for (const file of asset.files ?? []) verify(file.path, file.sha256);
const notices = read("legal/THIRD-PARTY-NOTICES.txt");
for (const file of ["packaging/windows/msix/THIRD-PARTY-NOTICES.txt", "website/public/legal/THIRD-PARTY-NOTICES.txt", "website-edge/public/legal/THIRD-PARTY-NOTICES.txt"]) verify(file, digest(notices));
if (inventory.missingNotices.length) throw new Error("Missing third-party notices block release.");
console.log(`Legal documents and notices verified: ${inventory.packages.length} components, ${inventory.textCount} preserved texts.`);
