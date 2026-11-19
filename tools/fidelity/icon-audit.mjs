// Icon audit: every glyph of `Derived` (kit/derived.slint) against the generated design paths.
// Prints matched / derived / absent; `node tools/fidelity/icon-audit.mjs [file.slint]`.
import { readFileSync } from "node:fs";
const ui = new URL("../../crates/wubble-ui/ui/", import.meta.url);
const gen = readFileSync(new URL("kit/gen/icons.slint", ui), "utf8");
const props = (text) => [...text.matchAll(/out property <string> ([\w-]+): "([^"]+)"/g)].map((m) => [m[1], m[2]]);
const norm = (d) => d.replace(/\s+/g, "").toLowerCase();
const design = new Map();
for (const [k, d] of props(gen)) if (!design.has(norm(d))) design.set(norm(d), k);
const src = process.argv[2] ?? "kit/derived.slint";
for (const [name, d] of props(readFileSync(new URL(src, ui), "utf8"))) {
  const hit = design.get(norm(d));
  console.log(`${hit ? "matched" : "differs"}\t${name}\t${hit ?? ""}`);
}
