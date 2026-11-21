// SPDX-License-Identifier: GPL-3.0-only
//
// Finding the wubble browser on this computer, without starting it and without the network.
// Pure where it can be: every function takes what it reads (environment, platform, a file
// reader) as arguments, so the tests run them on fixtures.
//
// The order: the manifest a hosting browser writes (`<runtime dir>/wubble/mcp/*.json`, see
// wubble's `crates/wubble-core/src/agent/manifest.rs`), then `wubble` on PATH, then the install
// locations of docs/release.md.

"use strict";

const path = require("node:path");

/// Where hosting browsers write their manifests (wubble's `mcp::manifest_dir`, in
/// `instance::private_root`): private to the user. `$XDG_RUNTIME_DIR`; else on macOS the
/// per-user temp directory (`$TMPDIR`, 0700); else `$XDG_STATE_HOME` or `~/.local/state`; on
/// Windows `%LOCALAPPDATA%`. With none of them known, the temp directory (the reader trusts only
/// a directory of this user's own there: `privateToMe`); null when even that is not absolute.
function manifestDir(env, platform, home, tmp) {
  const abs = (p) => (typeof p === "string" && path.isAbsolute(p) ? p : null);
  let root;
  if (platform === "win32") {
    root = abs(env.LOCALAPPDATA);
  } else if (abs(env.XDG_RUNTIME_DIR)) {
    root = env.XDG_RUNTIME_DIR;
  } else if (platform === "darwin") {
    root = null;
  } else {
    root = abs(env.XDG_STATE_HOME) || (abs(home) && path.join(home, ".local/state"));
  }
  root = root || abs(tmp);
  return root ? path.join(root, "wubble", "mcp") : null;
}

/// Whether an `lstat` result is a `kind` ("dir", "file", "socket"), not a link, owned by `uid`
/// with no group or other access. Without a uid (Windows) the location alone is trusted.
function privateToMe(stat, kind, uid) {
  if (!stat) return false;
  if (uid === undefined) return true;
  const rightKind =
    (kind === "dir" && stat.isDirectory()) ||
    (kind === "file" && stat.isFile()) ||
    (kind === "socket" && stat.isSocket());
  return rightKind && stat.uid === uid && (stat.mode & 0o077) === 0;
}

/// One manifest's text, or null when it is not one.
function parseManifest(text) {
  let v;
  try {
    v = JSON.parse(text);
  } catch {
    return null;
  }
  if (
    !v ||
    typeof v.pid !== "number" ||
    typeof v.profile !== "string" ||
    typeof v.socket !== "string" ||
    !Array.isArray(v.command) ||
    v.command.length === 0 ||
    !v.command.every((c) => typeof c === "string")
  ) {
    return null;
  }
  return {
    pid: v.pid,
    profile: v.profile,
    defaultProfile: v.defaultProfile === true,
    socket: v.socket,
    command: v.command,
    version: typeof v.version === "string" ? v.version : "",
  };
}

/// The manifests of browsers that still run (`alive(pid)`), from `files` (name -> text).
function liveManifests(files, alive) {
  return files
    .map(parseManifest)
    .filter((m) => m !== null && alive(m.pid))
    .sort((a, b) => a.profile.localeCompare(b.profile));
}

/// The binary a manifest names: the browser that wrote it.
function binaryOfManifests(live) {
  const preferred = live.find((m) => m.defaultProfile) || live[0];
  return preferred ? preferred.command[0] : null;
}

/// Where an install puts the binary (docs/release.md "Install layout"), in the order to try.
function installLocations(platform, home) {
  if (platform === "darwin") {
    return [
      "/Applications/wubble.app/Contents/MacOS/wubble",
      path.join(home, "Applications/wubble.app/Contents/MacOS/wubble"),
    ];
  }
  if (platform === "win32") {
    return [];
  }
  return [
    path.join(home, ".local/bin/wubble"),
    path.join(home, ".local/share/wubble/current/wubble"),
  ];
}

/// `wubble` on PATH, as `which` finds it.
function onPath(env, platform, isFile) {
  const sep = platform === "win32" ? ";" : ":";
  const names = platform === "win32" ? ["wubble.exe"] : ["wubble"];
  for (const dir of (env.PATH || "").split(sep)) {
    if (!dir) continue;
    for (const name of names) {
      const p = path.join(dir, name);
      if (isFile(p)) return p;
    }
  }
  return null;
}

/// The binary to run: the manifests', PATH's, an install location's; null when none.
function findBinary({ live, env, platform, home, isFile }) {
  return (
    binaryOfManifests(live) ||
    onPath(env, platform, isFile) ||
    installLocations(platform, home).find(isFile) ||
    null
  );
}

module.exports = {
  manifestDir,
  privateToMe,
  parseManifest,
  liveManifests,
  binaryOfManifests,
  installLocations,
  onPath,
  findBinary,
};
