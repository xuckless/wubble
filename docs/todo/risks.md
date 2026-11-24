# Risks

Part of the roadmap ([TODO.md](../../TODO.md)).

1. Slint's wgpu texture import is an `unstable-` feature and can change on any minor: pin versions.
2. Own glass and backdrop must fit the 8.33 ms frame beside Chromium's GPU work; the quality ladder is the fallback.
3. Prebuilt CEF is not ungoogled and has no proprietary codecs until M6.
4. `welding` pins CEF; every CEF upgrade is ours to do in the fork.
5. Extensions under windowless CEF need our patch queue (`patches/chromium`), rebuilt with every CEF upgrade and not yet built for macOS or Windows.
6. OSR costs one extra composite per frame and bypasses direct scanout for video.
7. An ungoogled + CEF patch queue rebased every Chromium milestone is permanent work for one person.
8. Windows is parked: its code is kept but neither built nor tried, so it decays.
9. Turso is pre-release: keep the file format SQLite-compatible and back up before migrations.
