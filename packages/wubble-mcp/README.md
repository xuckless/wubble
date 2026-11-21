# wubble-mcp

Let an AI agent (Claude Code, Codex, Gemini CLI, Cursor, opencode, Zed, ...) use the
[wubble](https://github.com/xuckless/wubble) browser on your computer, over the Model Context Protocol (MCP).

This package is a small launcher. It finds the wubble you installed and runs its built-in relay, `wubble mcp`,
which speaks MCP on stdio and passes it to the running browser over a socket only your user can open. It has no
dependencies, makes no network request, never starts the browser and never edits any app's configuration.

## Before you start

1. Install wubble and start it.
2. In wubble, open **Settings > Agents (MCP)** and turn on **Host wubble's MCP server**. It is off by default;
   while it is off, nothing is hosted and no agent can reach wubble.

Turning hosting on also adds wubble to the agent apps wubble finds on your computer (Settings lists them, and you
can remove any). You need this package only for an app wubble did not set up, or if you prefer `npx`.

## Add it to your agent app

Any app that takes an MCP server as a command:

```json
{
  "mcpServers": {
    "wubble": { "command": "npx", "args": ["-y", "wubble-mcp"] }
  }
}
```

| App | How |
|---|---|
| Claude Code | `claude mcp add wubble -- npx -y wubble-mcp` |
| Codex | `codex mcp add wubble -- npx -y wubble-mcp`, and in `~/.codex/config.toml` under `[mcp_servers.wubble]`: `tool_timeout_sec = 3600` |
| Gemini CLI | `gemini mcp add -s user wubble npx -y wubble-mcp`, then `"timeout": 3600000` on its entry in `~/.gemini/settings.json` |
| Cursor | the JSON above in `~/.cursor/mcp.json` |
| Claude desktop app | the JSON above in `claude_desktop_config.json` |
| Antigravity | the JSON above in `~/.gemini/config/mcp_config.json` |
| opencode | in `opencode.json`: `"mcp": { "wubble": { "type": "local", "command": ["npx", "-y", "wubble-mcp"] } }` |
| Zed | in `settings.json`: `"context_servers": { "wubble": { "command": "npx", "args": ["-y", "wubble-mcp"] } }` |
| VS Code | in `mcp.json`: `"servers": { "wubble": { "type": "stdio", "command": "npx", "args": ["-y", "wubble-mcp"] } }` |

Running several wubble profiles? Add `--profile DIR` to the arguments to choose one; `npx wubble-mcp --list`
shows the browsers that are hosting.

## Every session needs your approval

When an agent starts a session, wubble shows an approval sheet: which sites the agent may visit, which saved
secrets it may use, and how it runs. Nothing happens until you press Allow, so the agent's first call
(`start_session`) can wait for minutes. Give the server a long tool timeout (the table shows how where the
default is short). You can stop every session from wubble at any time; Settings > Agents (MCP) links to the
activity log.

Being findable is not access: the socket is private to your user, never a network port, and every session
needs your press.

## Troubleshooting

The agent sees these as MCP errors; the same line goes to stderr.

| Message | What to do |
|---|---|
| wubble is not installed | Install wubble, or put `wubble` on your PATH. |
| wubble is not hosting its MCP server | Start wubble and turn hosting on in Settings > Agents (MCP). |
| several wubble browsers host MCP | Add `--profile DIR` for the one you want (`--list` names them). |
| the wubble at ... is too old | Update wubble (Settings > Updates). |
| a call times out while you approve | Raise the app's tool timeout (see the table above). |

`npx wubble-mcp --list` prints each hosting browser's profile, socket, version and process id.

## Licence

GPL-3.0-only, as wubble.
