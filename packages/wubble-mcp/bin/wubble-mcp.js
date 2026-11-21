#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-only
//
// wubble-mcp: runs the installed wubble's `wubble mcp` (the one relay implementation) with the
// arguments given, its stdio passed straight through. When there is no wubble to run, every
// request on stdin is answered with an MCP error that says what to do, and the same line goes
// to stderr. It never starts the browser, never touches a harness's configuration and makes no
// network request.

"use strict";

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const readline = require("node:readline");
const { spawn, spawnSync } = require("node:child_process");

const find = require("../lib/find.js");
const { USAGE, parseArgs, ERRORS, errorReply } = require("../lib/cli.js");

function isFile(p) {
  try {
    return fs.statSync(p).isFile();
  } catch {
    return false;
  }
}

function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return e.code === "EPERM";
  }
}

function lstat(p) {
  try {
    return fs.lstatSync(p);
  } catch {
    return null;
  }
}

/// The manifests of live browsers, trusted only when the directory, the file and its socket
/// are this user's own and private.
function readManifests() {
  const uid = typeof process.getuid === "function" ? process.getuid() : undefined;
  const dir = find.manifestDir(process.env, process.platform, os.homedir());
  if (!dir || !find.privateToMe(lstat(dir), "dir", uid)) return [];
  let names = [];
  try {
    names = fs.readdirSync(dir).filter((n) => n.endsWith(".json"));
  } catch {
    return [];
  }
  const texts = names.flatMap((n) => {
    const p = path.join(dir, n);
    if (!find.privateToMe(lstat(p), "file", uid)) return [];
    try {
      return [fs.readFileSync(p, "utf8")];
    } catch {
      return [];
    }
  });
  return find
    .liveManifests(texts, alive)
    .filter((m) => process.platform === "win32" || find.privateToMe(lstat(m.socket), "socket", uid));
}

/// Answers every request with `message` until stdin ends; the line also goes to stderr.
function serveError(message) {
  process.stderr.write(`wubble-mcp: ${message}\n`);
  const rl = readline.createInterface({ input: process.stdin });
  rl.on("line", (line) => {
    const reply = errorReply(line, message);
    if (reply) process.stdout.write(reply + "\n");
  });
  rl.on("close", () => process.exit(1));
}

function main() {
  const parsed = parseArgs(process.argv.slice(2));
  if (parsed.help) {
    process.stdout.write(USAGE);
    return;
  }
  if (parsed.version) {
    process.stdout.write(require("../package.json").version + "\n");
    return;
  }
  if (parsed.error) {
    process.stderr.write(`wubble-mcp: ${parsed.error}\n\n${USAGE}`);
    process.exit(2);
  }
  const live = readManifests();
  const bin = find.findBinary({
    live,
    env: process.env,
    platform: process.platform,
    home: os.homedir(),
    isFile,
  });
  const listing = parsed.args.includes("--list");
  const fail = (message) => {
    if (!listing) return serveError(message);
    process.stderr.write(`wubble-mcp: ${message}\n`);
    process.exit(1);
  };
  if (!bin) return fail(ERRORS.notInstalled());
  // a browser that writes manifests is new enough; otherwise ask the binary
  if (live.length === 0) {
    const help = spawnSync(bin, ["mcp", "--help"], { encoding: "utf8", timeout: 10000 });
    if (help.error) return fail(ERRORS.cannotRun(bin, help.error.message));
    if (!`${help.stdout}${help.stderr}`.includes("--list")) return fail(ERRORS.tooOld(bin));
  }
  // not running, hosting off, several browsers: `wubble mcp` answers those itself
  const child = spawn(bin, ["mcp", ...parsed.args], { stdio: "inherit" });
  child.on("error", (e) => serveError(ERRORS.cannotRun(bin, e.message)));
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.on(signal, () => child.kill(signal));
  }
  child.on("exit", (code, signal) => process.exit(code ?? (signal ? 1 : 0)));
}

main();
