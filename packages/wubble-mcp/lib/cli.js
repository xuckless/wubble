// SPDX-License-Identifier: GPL-3.0-only
//
// The command line and the errors that teach. Pure: no process, no files.

"use strict";

const USAGE = `wubble-mcp: wubble's MCP server for agent harnesses (Claude Code, Codex, Gemini CLI, ...).

Usage: wubble-mcp [--profile DIR] [--list]

Runs the installed wubble's relay, \`wubble mcp\`, which speaks MCP on stdio and relays to the
running browser's private socket. It finds the browser by itself; it never starts it and makes no
network request. Turn hosting on in wubble: Settings > Agents (MCP). Every session needs your
approval in wubble, so the first call (start_session) can block for minutes: give the server a
long tool timeout.

  --profile DIR   the browser's profile, to choose one of several hosting browsers
  --list          print the hosting browsers and exit
  -h, --help      this text
  -v, --version   this package's version
`;

/// What the arguments ask for: `{help}`, `{version}`, `{error}`, or `{args}` to pass on.
function parseArgs(argv) {
  const args = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "-h" || a === "--help") return { help: true };
    if (a === "-v" || a === "--version") return { version: true };
    if (a === "--list") {
      args.push(a);
    } else if (a === "--profile") {
      const dir = argv[i + 1];
      if (!dir || dir.startsWith("-")) return { error: "--profile needs a directory" };
      args.push(a, dir);
      i++;
    } else if (a.startsWith("--profile=")) {
      const dir = a.slice("--profile=".length);
      if (!dir) return { error: "--profile needs a directory" };
      args.push("--profile", dir);
    } else {
      return { error: `unknown argument: ${a}` };
    }
  }
  if (args.includes("--list") && args.includes("--profile")) {
    return { error: "--list and --profile do not go together" };
  }
  return { args };
}

const ERRORS = {
  notInstalled: () =>
    "wubble is not installed (not found from a hosting browser's manifest, on PATH, or in its " +
    "install locations): install it from https://github.com/xuckless/wubble, start it, and turn " +
    "hosting on in Settings > Agents (MCP)",
  tooOld: (bin) =>
    `the wubble at ${bin} is too old for wubble-mcp (its \`wubble mcp\` has no --list): update ` +
    "it (Settings > Updates), then turn hosting on in Settings > Agents (MCP)",
  cannotRun: (bin, why) => `cannot run ${bin}: ${why}`,
};

/// The JSON-RPC error answer to one line of a client's input, or null for a notification or
/// a line that is not a request.
function errorReply(line, message) {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return null;
  }
  if (!msg || typeof msg !== "object" || msg.id === undefined || msg.id === null || !msg.method) {
    return null;
  }
  return JSON.stringify({ jsonrpc: "2.0", id: msg.id, error: { code: -32000, message } });
}

module.exports = { USAGE, parseArgs, ERRORS, errorReply };
