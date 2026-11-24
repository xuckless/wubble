# Scroll lag on the macOS store test build

Shelved by the owner (2026-11-23): "I'll point it out when I find it".

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

- [ ] A scroll lag seen once on the store test build on macOS (trackpad, 120 Hz, no flag), not reproducible; a read of the diff found no cause. First check when it returns: `--no-begin-frames`, and the build and launch differences of the unsigned test app
