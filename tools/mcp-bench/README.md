# mcp-bench

A hand-run workflow benchmark for macOS: drives the installed wubble through its own MCP server
(an agent session the owner approves on screen) while sampling every wubble process. Results go in
[docs/measurements.md](../../docs/measurements.md) ("Workflow on macOS"). Nothing here runs in
`cargo test` or CI.

| File | Does |
|---|---|
| `mcpd.py` | Holds one `wubble mcp` connection open; serves tool calls on a unix socket |
| `mcpc.py` | One tool call from the shell (`start_session`, `list_tabs`, ...) |
| `sampler.sh` | Every ~2 s: CPU and memory per process (`top`), each process's role, GPU utilisation (`ioreg`), free memory |
| `workflow.py` | The workflow: idle, 10 sites opened and scrolled, one video, fullscreen, three videos, switching, browsing over background video, paused, closed |
| `analyze.py` | A Markdown table by phase: browser process, GPU process, renderers, helpers |

## Run

Use a short directory: a unix socket path must stay under 104 bytes.

    cd tools/mcp-bench
    python3 -I mcpd.py s.sock > mcpd.log 2>&1 &
    bash sampler.sh data &
    python3 -I mcpc.py s.sock start_session '{"purpose": "Workflow benchmark", "headless": false,
      "sites": ["youtube.com", "*.youtube.com", "github.com", "*.github.com", "wikipedia.org",
      "*.wikipedia.org", "developer.mozilla.org", "news.ycombinator.com", "bbc.com", "*.bbc.com",
      "*.bbc.co.uk", "theverge.com", "*.theverge.com", "docs.rs", "rust-lang.org", "*.rust-lang.org",
      "apple.com", "*.apple.com"]}'
    # approve the sheet in wubble; the reply names the session and its first tab
    python3 -I workflow.py s.sock SESSION TAB data      # about 12 minutes
    python3 -I analyze.py data
    python3 -I mcpc.py s.sock stop_session '{"session": "SESSION"}'
    kill %1 %2

Close or pause other media first; other apps' load shows in the GPU column (the whole GPU).
`top`'s memory counts GPU buffers, so it reads above `footprint -p PID`.
