// SPDX-License-Identifier: GPL-3.0-only
"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const find = require("../lib/find.js");

const manifest = (profile, defaultProfile, pid = 7) =>
  JSON.stringify({
    name: "wubble",
    version: "0.1.0",
    pid,
    profile,
    defaultProfile,
    socket: `${profile}/mcp.sock`,
    transport: "stdio",
    command: [`/opt/${profile.slice(1)}/wubble`, "mcp"],
    protocolVersions: ["2026-07-28", "2025-11-25"],
  });

test("the manifests are where wubble writes them", () => {
  const dir = (env, platform) => find.manifestDir(env, platform, "/home/u", "/tmp/me");
  assert.equal(dir({ XDG_RUNTIME_DIR: "/run/user/1" }, "linux"), "/run/user/1/wubble/mcp");
  assert.equal(dir({}, "linux"), "/home/u/.local/state/wubble/mcp");
  assert.equal(dir({ XDG_STATE_HOME: "/s" }, "linux"), "/s/wubble/mcp");
  // macOS: the per-user $TMPDIR, where wubble's sockets have always been
  assert.equal(dir({}, "darwin"), "/tmp/me/wubble/mcp");
  assert.equal(dir({ XDG_RUNTIME_DIR: "relative" }, "linux"), "/home/u/.local/state/wubble/mcp");
  // no home: the temp directory, as wubble; never a relative path
  assert.equal(find.manifestDir({}, "linux", "", "/tmp/me"), "/tmp/me/wubble/mcp");
  assert.equal(find.manifestDir({}, "linux", "", "rel"), null);
});

test("only this user's private manifests are trusted", () => {
  const stat = (kind, uid, mode) => ({
    isDirectory: () => kind === "dir",
    isFile: () => kind === "file",
    isSocket: () => kind === "socket",
    uid,
    mode,
  });
  assert.ok(find.privateToMe(stat("dir", 7, 0o40700), "dir", 7));
  assert.ok(!find.privateToMe(stat("dir", 7, 0o40755), "dir", 7));
  assert.ok(!find.privateToMe(stat("file", 8, 0o100600), "file", 7));
  assert.ok(!find.privateToMe(stat("file", 7, 0o100600), "socket", 7));
  assert.ok(!find.privateToMe(null, "file", 7));
});

test("a manifest reads back; anything else is not one", () => {
  const m = find.parseManifest(manifest("/p", true));
  assert.deepEqual(m, {
    pid: 7,
    profile: "/p",
    defaultProfile: true,
    socket: "/p/mcp.sock",
    command: ["/opt/p/wubble", "mcp"],
    version: "0.1.0",
  });
  assert.equal(find.parseManifest("{"), null);
  assert.equal(find.parseManifest('{"pid": "7"}'), null);
  assert.equal(find.parseManifest(JSON.stringify({ pid: 1, profile: "/p", socket: "s", command: [] })), null);
});

test("only live browsers count, and the default profile's binary is preferred", () => {
  const files = [manifest("/b", false, 2), manifest("/a", true, 3), manifest("/dead", false, 4), "junk"];
  const live = find.liveManifests(files, (pid) => pid !== 4);
  assert.deepEqual(live.map((m) => m.profile), ["/a", "/b"]);
  assert.equal(find.binaryOfManifests(live), "/opt/a/wubble");
  assert.equal(find.binaryOfManifests([]), null);
});

test("the binary: manifests, then PATH, then the install locations", () => {
  const files = new Set(["/usr/bin/wubble", "/home/u/.local/bin/wubble"]);
  const isFile = (p) => files.has(p);
  const env = { PATH: "/nope:/usr/bin" };
  const live = find.liveManifests([manifest("/a", true)], () => true);
  assert.equal(find.findBinary({ live, env, platform: "linux", home: "/home/u", isFile }), "/opt/a/wubble");
  assert.equal(find.findBinary({ live: [], env, platform: "linux", home: "/home/u", isFile }), "/usr/bin/wubble");
  assert.equal(
    find.findBinary({ live: [], env: { PATH: "" }, platform: "linux", home: "/home/u", isFile }),
    "/home/u/.local/bin/wubble",
  );
  assert.equal(find.findBinary({ live: [], env: {}, platform: "linux", home: "/x", isFile }), null);
  assert.deepEqual(find.installLocations("darwin", "/Users/u"), [
    "/Applications/wubble.app/Contents/MacOS/wubble",
    "/Users/u/Applications/wubble.app/Contents/MacOS/wubble",
  ]);
});
