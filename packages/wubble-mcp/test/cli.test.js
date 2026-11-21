// SPDX-License-Identifier: GPL-3.0-only
"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { parseArgs, ERRORS, errorReply, USAGE } = require("../lib/cli.js");

test("the arguments pass on to `wubble mcp`", () => {
  assert.deepEqual(parseArgs([]), { args: [] });
  assert.deepEqual(parseArgs(["--list"]), { args: ["--list"] });
  assert.deepEqual(parseArgs(["--profile", "/p"]), { args: ["--profile", "/p"] });
  assert.deepEqual(parseArgs(["--profile=/p"]), { args: ["--profile", "/p"] });
  assert.ok(parseArgs(["--profile"]).error);
  assert.ok(parseArgs(["--list", "--profile", "/p"]).error);
  assert.ok(parseArgs(["--launch"]).error.includes("--launch"));
  assert.deepEqual(parseArgs(["-h"]), { help: true });
  assert.ok(USAGE.includes("Settings > Agents (MCP)"));
});

test("requests get an error that teaches; notifications get nothing", () => {
  const reply = JSON.parse(errorReply('{"jsonrpc":"2.0","id":3,"method":"initialize"}', ERRORS.notInstalled()));
  assert.equal(reply.id, 3);
  assert.equal(reply.error.code, -32000);
  assert.match(reply.error.message, /not installed.*Settings > Agents \(MCP\)/);
  assert.equal(errorReply('{"jsonrpc":"2.0","method":"notifications/initialized"}', "x"), null);
  assert.equal(errorReply("not json", "x"), null);
  assert.match(ERRORS.tooOld("/w"), /too old.*Settings > Updates/);
});
