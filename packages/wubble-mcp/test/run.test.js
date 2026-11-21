// SPDX-License-Identifier: GPL-3.0-only
// The bin end to end with a stand-in `wubble` on PATH (a shell script): the arguments reach it,
// and with no wubble anywhere every request gets the teaching error.
"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const BIN = path.join(__dirname, "..", "bin", "wubble-mcp.js");

function scratch() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "wubble-mcp-test-"));
  return { dir, env: { PATH: path.join(dir, "bin"), HOME: dir, XDG_RUNTIME_DIR: path.join(dir, "run") } };
}

test("runs `wubble mcp` with the arguments given", { skip: process.platform === "win32" }, () => {
  const { dir, env } = scratch();
  fs.mkdirSync(path.join(dir, "bin"));
  const fake = path.join(dir, "bin", "wubble");
  fs.writeFileSync(fake, '#!/bin/sh\nif [ "$2" = "--help" ]; then echo "--list"; exit 0; fi\necho "args: $*"\n');
  fs.chmodSync(fake, 0o755);
  const run = spawnSync(process.execPath, [BIN, "--profile", "/p"], { env, encoding: "utf8" });
  assert.equal(run.status, 0, run.stderr);
  assert.equal(run.stdout, "args: mcp --profile /p\n");
});

test("an old wubble is named as too old", { skip: process.platform === "win32" }, () => {
  const { dir, env } = scratch();
  fs.mkdirSync(path.join(dir, "bin"));
  const fake = path.join(dir, "bin", "wubble");
  fs.writeFileSync(fake, "#!/bin/sh\necho 'Usage: wubble mcp [--profile DIR]'\n");
  fs.chmodSync(fake, 0o755);
  const run = spawnSync(process.execPath, [BIN], {
    env,
    encoding: "utf8",
    input: '{"jsonrpc":"2.0","id":1,"method":"initialize"}\n',
  });
  assert.equal(run.status, 1);
  assert.match(JSON.parse(run.stdout).error.message, /too old/);
  assert.match(run.stderr, /too old/);
});

test("with no wubble every request is answered with what to do", () => {
  const { env } = scratch();
  const run = spawnSync(process.execPath, [BIN], {
    env,
    encoding: "utf8",
    input:
      '{"jsonrpc":"2.0","id":1,"method":"initialize"}\n' +
      '{"jsonrpc":"2.0","method":"notifications/initialized"}\n' +
      '{"jsonrpc":"2.0","id":"b","method":"tools/list"}\n',
  });
  const lines = run.stdout.trim().split("\n").map((l) => JSON.parse(l));
  assert.deepEqual(lines.map((l) => l.id), [1, "b"]);
  assert.match(lines[0].error.message, /not installed/);
  assert.match(run.stderr, /not installed/);
  const list = spawnSync(process.execPath, [BIN, "--list"], { env, encoding: "utf8" });
  assert.equal(list.status, 1);
  assert.match(list.stderr, /not installed/);
});
