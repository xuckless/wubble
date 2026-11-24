# Windows (parked)

Part of the roadmap ([TODO.md](../../TODO.md)). Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

Not a target for now (owner, 2026-11-23): no CI job, no release; the code is kept and stays syntactically sane.
- [ ] Windows: the agent apps' paths (`agent::harness` lists Linux and macOS only) and the manifest's runtime dir
- [ ] Windows CI: the Test step now finishes and 4 tests of the `wubble` binary fail there (run 44988591703, 2026-11-22): `instance::tests::the_private_root_by_platform`, `mcp::tests::a_connection_answers_discovery_and_hands_calls_to_the_host`, `mcp::tests::a_stop_closes_open_connections_and_ids_never_repeat`, `updater::install::tests::linux_stages_switches_current_and_rolls_back` (the last is a Linux-only test not gated)
- [ ] Drag out on Windows (the stub: `wl::Null`)
- [ ] Windows sandbox: restructure as a client DLL behind CEF's `bootstrap.exe`
