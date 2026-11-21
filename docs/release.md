# Releases

How wubble ships: a tarball and `install.sh` on Linux, a signed and notarised DMG and the same
script on macOS (docs/decisions.md "Release form"), updated in place by the app itself
("Auto-update", "Update signing"). The first releases are previews (`0.1.0-preview.N`).

## Where things are

- Tags `v<version>` on GitHub Releases of `xuckless/wubble`; each release holds
  `wubble-<V>-<platform>.tar.xz` (platforms `linux-x86_64`, `macos-aarch64`), the Linux
  `.debug.tar.xz` (split symbols), the `.dmg`, a `.sha256` and a `.minisig` for each, and
  `latest.json` (+ `.minisig`), the signed update manifest.
- The rolling release `updates` (a prerelease, never "latest") holds `preview.json` and, from the
  first stable version on, `stable.json`, each with its `.minisig`: byte copies of the newest
  `latest.json`. The app and `install.sh` read only these.
- Manifest schema and checks: `crates/wubble-core/src/update/manifest.rs`; install layout and
  rollback: `crates/wubble-core/src/update/install.rs`; the tool: `crates/wubble-release`.

## One-time key setup

1. Make the update key pair somewhere outside the repo, with a password:
   `WUBBLE_KEY_PW=... cargo run -p wubble-release -- keygen --out ~/wubble-keys --password-env WUBBLE_KEY_PW`
   (or `minisign -G -p wubble-update.pub -s wubble-update.key`; the formats are the same).
   Back up `wubble-update.key` and its password offline; a lost key means users must reinstall.
2. Give CI the secret: `gh secret set WUBBLE_UPDATE_KEY < ~/wubble-keys/wubble-update.key`
   and `gh secret set WUBBLE_UPDATE_KEY_PASSWORD` (the password).
3. Put the public key (the base64 line `keygen` prints, the second line of `wubble-update.pub`)
   into `PUBLIC_KEY` in `crates/wubble-core/src/update/key.rs` and into the `PUBLIC_KEY='...'`
   line of `scripts/release/install.sh`; `cargo test -p wubble-release` checks they agree.
   Commit. Until then the app's updater is inert (Settings says "Updates are not set up in this
   build") and `install.sh` checks only the sha256 (with `--require-signature` it refuses).

## Each release

1. Set `[workspace.package] version` in `Cargo.toml` (e.g. `0.1.0-preview.1`), commit, tag
   `v<version>`, push the commit and the tag.
2. Linux (this box): `CEF_PATH=<codec CEF> scripts/release/package-linux.sh`
   (the own CEF build with codecs; the script refuses to pack without `CEF_PATH`). It writes
   `dist/wubble-<V>-linux-x86_64.tar.xz`, the `.debug.tar.xz` and their `.sha256`.
3. macOS (the Mac): `CEF_PATH=<codec CEF for macOS> WUBBLE_SIGN_IDENTITY="Developer ID Application: ..."
   WUBBLE_NOTARY_PROFILE=<profile> scripts/release/package-macos.sh` (store the profile once with
   `xcrun notarytool store-credentials`). It signs inside out with
   `scripts/release/macos/entitlements.plist`, notarises and staples the app and the DMG, and
   writes `dist/wubble-<V>-macos-aarch64.tar.xz`, `.dmg` and their `.sha256`.
4. Sign on this box (the key never leaves it: `~/.config/wubble-release/`), with the key's text and
   its password in two environment variables:
   `wubble-release sign --key-env K --password-env P dist/wubble-<V>-<platform>.tar.xz`, then
   `wubble-release make-manifest --version <V> --base-url https://github.com/xuckless/wubble/releases/download/v<V> --out dist/latest.json dist`
   and sign `dist/latest.json` the same way.
5. Test before publishing: a folder shaped like the releases (`v<V>/...`, `updates/preview.json`)
   and `WUBBLE_RELEASES=file://<folder> sh scripts/release/install.sh` in a throwaway `HOME`: a
   fresh install, an install over the previous version, a tampered archive refused (by its sha256;
   by its signature too with `minisign` on `PATH`), once with and once without `minisign` on `PATH`
   (without it: one warning, the sha256 alone; `--require-signature` refuses); then the
   installed binary's `--smoke boot` and `--smoke page`. Smokes that find elements by id
   (`mcp`, `settings`, `tabs`, `video`) need a `dist` build with `SLINT_EMIT_DEBUG_INFO=1`.
6. Publish (the owner's go, every release): push the commit and the tag;
   `gh release create v<V> --prerelease --verify-tag --title "wubble <V>" --notes-file ... dist/wubble-<V>-* dist/latest.json*`;
   then copy `latest.json` and its `.minisig` to the rolling `updates` release as `preview.json`
   (`gh release upload updates ... --clobber`): from then on installs offer the update.
7. Check: a fresh `install.sh` from GitHub gives `<V>`; an install of the previous version, run
   once, stages `<V>` (`updates/launch.json`) and starts as it the next time.
8. macOS, after the tag is pushed: the owner runs `package-macos.sh` on the Mac (step 3); its
   archive is signed here, added to `latest.json` (`make-manifest` again with both archives in
   `dist/`), and the manifest, its signature and the Mac files are uploaded; `preview.json` is
   replaced again. `.github/workflows/release-sign.yml` (signing in CI) is not used.

## What ships in each bundle

- `LICENSE` (GPL-3.0-only) and `SOURCE.md`: the written source offer, naming the tag of the exact
  source and the CEF build it embeds.
- `THIRD_PARTY_NOTICES.md` (code ported into wubble) and Chromium/CEF's `CREDITS.html` (the
  licences of everything inside libcef). macOS: all four in `wubble.app/Contents/Resources`.
- `release.json` (`{"version","platform","commit"}`): the app counts an install as its own only
  when this file is there and the bundle sits in the layout below.
- Codecs: release builds use the own CEF build with proprietary codecs (H.264, AAC, MPEG-TS, HLS;
  not HEVC on Linux), made by `scripts/build-cef.sh` (docs/research/cef-codec-build.md). The owner
  accepted the patent exposure (docs/decisions.md "Release codecs"). The macOS codec CEF is not
  built yet: until it is, package-macos.sh has nothing to point `CEF_PATH` at.

## Install layout

- Linux: `~/.local/share/wubble/versions/<V>/` (the whole bundle), `current` linking to one of
  them (switched by a rename), `~/.local/bin/wubble` linking to `current/wubble`, a desktop entry
  and icons in the XDG dirs, updater state in `~/.local/share/wubble/updates/`. The same folder
  holds the profile; `install.sh --uninstall` removes only the parts above.
- macOS: `/Applications/wubble.app` (or `~/Applications` when `/Applications` is not writable),
  state in `~/Library/Application Support/wubble/updates/`.
- A development build (run from `target/`) or a system package never updates itself; Settings
  says "updates are managed elsewhere".

## What the app sends

When wubble opens, at most once every 6 hours and only with Settings > Updates > "Check for
updates when wubble opens" on (the default): a GET of `updates/<channel>.json` and its
`.minisig`, then, for a newer release, the archive. No identifiers, no cookies,
`User-Agent: wubble`, the proxy from `HTTPS_PROXY`/`ALL_PROXY`. "Check now" asks at once.

## Updating and rollback

The download is checked (size, SHA-256 from the signed manifest, its own signature) before it is
unpacked, unpacked beside the running version, checked again (`release.json`), and made the one the
next launch runs; Settings and a toast offer "Restart to update". A new version that fails to start
3 times (no 20 s healthy run) falls back to the previous one, which is never offered again; after a
healthy start, versions older than the previous one are deleted.

## The npm package `wubble-mcp`

`packages/wubble-mcp/` (outside the Cargo workspace, no dependencies, no install scripts) is a launcher for agent
apps that prefer `npx -y wubble-mcp`: it runs the installed browser's `wubble mcp`, found from a hosting browser's
manifest, then PATH, then the install locations above, and passes `--profile` and `--list` on. It never starts the
browser, never edits an app's configuration and makes no request. Not published yet; Rust CI and the release
scripts do not build or test it.

Versioning: its own semver, independent of wubble's. A patch for a launcher fix, a minor when it learns a new flag
or location, a major only if the command line changes. It needs a wubble whose `wubble mcp --help` lists `--list`
(older ones are named "too old").

Publishing, by the owner, from the owner's npm account:

1. `cd packages/wubble-mcp && node --test` (unit tests and the bin against stand-in binaries).
2. Bump `version` in `package.json`; commit.
3. `npm pack`, then from an empty directory with a hosting wubble running:
   `npx --yes --package=/abs/wubble-mcp-<V>.tgz wubble-mcp --list` names it, and an `initialize` piped in gets
   an answer.
4. `npm login` (the owner's account, with two-factor), then `npm publish --access public` in `packages/wubble-mcp`.
5. Check `npm view wubble-mcp` shows the version, and tag the commit `wubble-mcp-v<V>`.
