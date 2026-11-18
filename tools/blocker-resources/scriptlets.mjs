// Writes uBlock Origin's scriptlets as adblock-rust resources. Run by hand when
// updating uBO; the output is committed. uBO's module is imported under Node's
// permission model (read access to the checkout only):
//
//   node --permission --allow-fs-read=<uBlock checkout> --allow-fs-read=tools \
//     tools/blocker-resources/scriptlets.mjs <uBlock checkout> \
//     > crates/wubble-blocker/data/resources/scriptlets.json
//
// Ported from brave/brave-core-crx-packager lib/adBlockRustUtils.js
// (`generateResources`, MPL-2.0): each entry of uBO's `builtinScriptlets`
// (gorhill/uBlock src/js/resources/scriptlets.js, GPL-3.0) becomes
// {name, aliases, kind, content: base64(fn source), dependencies}.
// wubble change: `requiresTrust` scriptlets get `permission: 1` (bit 0 = uBO's
// own lists), so only lists parsed with that bit can inject them; `.fn`
// helpers get adblock's `fn/javascript` kind.
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const root = process.argv[2]
if (!root) {
  console.error('usage: scriptlets.mjs <uBlock checkout>')
  process.exit(2)
}
const module = path.resolve(root, 'src/js/resources/scriptlets.js')
const { builtinScriptlets } = await import(pathToFileURL(module).href)

const names = new Set(builtinScriptlets.map(s => s.name))
for (const s of builtinScriptlets) {
  for (const dep of s.dependencies ?? []) {
    if (!names.has(dep)) {
      console.error(`uBO scriptlet ${s.name} is missing dependency ${dep}`)
      process.exit(1)
    }
  }
}

const resources = builtinScriptlets.map(s => {
  const r = {
    name: s.name,
    aliases: s.aliases ?? [],
    kind: { mime: s.name.endsWith('.fn') ? 'fn/javascript' : 'application/javascript' },
    content: Buffer.from(s.fn.toString()).toString('base64'),
    dependencies: s.dependencies ?? []
  }
  if (s.requiresTrust) r.permission = 1
  return r
})
resources.sort((a, b) => a.name.localeCompare(b.name))
process.stdout.write(JSON.stringify(resources) + '\n')
