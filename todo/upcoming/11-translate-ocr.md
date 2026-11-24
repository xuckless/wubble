# 11 Translation and OCR

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

## After the optimization pass (owner, 2026-11-21)
Built as planned in the research, each as its own pass; nothing downloads or asks a server unless the owner turns it on.
- [ ] Page translation, on-device: the pipeline in [docs/research/translate.md](../../docs/research/translate.md) (build-out `translate-detect-offer`, `translate-bergamot-backend`, `translate-selection-reader`). First, spikes of about 1.5 days: does Chrome's `Translator` API answer in CEF's windowless mode (`View::eval` of `Translator.availability`; expected not), Bergamot's HTML and alignment API, the model registry and each model's licence (the models repo is archived). Then a `translate` frame channel and walker script (the cosmetic channel's pattern), `wubble_core::translate` (state, `whatlang` detection), a helper process running bergamot-translator; about 17 days in all
- [ ] Live text in images, on-device OCR: the pipeline in [docs/research/ocr.md](../../docs/research/ocr.md)
- [ ] Preferred languages: Accept-Language and the page locale (nothing sets them; translation and spell languages need it first) (scout, 2026-11-22: features no plan has yet)

## Build-out features (2026-11-22)

## Page translation: language detection, the offer, rules, state machine and in-page DOM translation (`translate-detect-offer`)

Key: `translate-detect-offer` · engine patch: no · state: not written.

TODO line 56, docs/research/translate.md sections 5-6 (Chrome's Translate bubble, built on-device). (1) Detection: a helper content script (like `crates/wubble-helper/src/*.js`) reports `<html lang>`, Content-Language and about 2 KB of visible text at DOMContentLoaded. Detect with a pure Rust crate (whatlang, MIT, or lingua; licence-checked, with a THIRD_PARTY_NOTICES entry). (2) A new `wubble_core::translate`: `TranslateState` (Idle, Offered, Downloading(pair), Translating(progress), Translated, Failed) and a pure `transition(state, event) -> (state, effects)`, unit-tested. Per-site and per-language always/never/ask rules are a settings table (`wubble_core::settings`, `wubble-store` settings). (3) The offer: a Translate chip by the address pill and a popover (source, target, always/never), built from kit parts (derived pieces go on docs/fidelity/derived-review.md). Context-menu rows "Translate to <lang>" and "Show original" go in `crates/wubble/src/menu.rs`. (4) The in-page walker script: viewport-ordered translation units, kept inline tags, a skip list (code, pre, inputs, contenteditable, translate=no, .notranslate), a WeakMap of originals, a rAF-coalesced MutationObserver, and show-original. Units go to a `TranslateBackend` trait (`backend-bergamot` feature implements it) through `View::eval`/frame messages. No backend is on by default.

## Page translation: Bergamot helper process and opt-in model downloads (`translate-bergamot-backend`)

Key: `translate-bergamot-backend` · engine patch: no · state: not written.

TODO line 56, docs/research/translate.md 3.1 and 6. Implement the `TranslateBackend` trait (from `translate-detect-offer`) with Mozilla's bergamot-translator (MPL-2.0) in a new utility binary `wubble-translate-helper` (a new crate under crates/, tiny, no network). It is a child of the browser and takes `{pair, units[]}` over a pipe as JSON lines, returning the translated units with their HTML alignment. Build it through `cxx` or a C shim. If the native build stalls, the research's fallback is allowed: Firefox's WASM Bergamot in a hidden wubble page. Models: a per-pair download manager (in wubble-engine or the binary) that fetches Mozilla's published model files with pinned SHA-256 hashes into `<profile>/translate/<pair>/`. It runs only after an explicit user action in the offer popover or Settings. A setting `translate.downloads` is off by default, as AGENTS.md requires no request by default. Show progress through `TranslateState::Downloading`. Settings → Languages: installed pairs, sizes, remove. THIRD_PARTY_NOTICES entries for the engine and each model's licence. Private profiles keep results in memory only. Linux and macOS first; Windows compiles in CI.

## Page translation: translate selection and reader-mode translation (`translate-selection-reader`)

Key: `translate-selection-reader` · engine patch: no · state: not written.

TODO line 56, docs/research/translate.md section 5 points 6-7 (Chrome's "Translate selection to <lang>" context row). Add a context-menu row on a text selection (`crates/wubble/src/menu.rs`, `wubble_core` menu model) that sends the selection text through the same `TranslateBackend` (feature `translate-bergamot-backend`), with no page mutation. Show the result in a read-only popover built from kit parts (a sheet or popover with copy). The reader view (`wubble_core::reader`, `crates/wubble/assets/reader.js`) runs the same unit walker on the extracted article after extraction, and remembers the last choice per site in the translate rules table. Reuse the `translate` state machine with a Selection target, no second machine. New core logic gets unit tests.

## Live text in images: OCR job machine, image fetch, cache and selectable text layer (`ocr-pipeline-text-layer`)

Key: `ocr-pipeline-text-layer` · engine patch: no · state: not written.

TODO line 57, docs/research/ocr.md sections 4-5 (Chrome's "Select text in image"/Lens-less, Safari Live Text). (1) A pure `wubble_core::ocr`: `OcrState` (Idle, Fetching, Recognising, Ready, Failed) with `transition` and effects, plus an LRU keyed by blake3(bytes), engine version and languages, unit-tested. The cache is memory only for private profiles. (2) Pixels: the original image bytes through the engine's own fetch on the tab's request context (`crates/wubble-engine/src/fetch.rs`), with DevTools `Page.captureScreenshot` with a clip for canvas, video frames and CSS backgrounds (`View::cdp`). No frame-texture readback. (3) Presentation: a page script makes an injected closed-shadow-root layer of transparent `<span>`s positioned over the image. It follows the image's box (ResizeObserver, getBoundingClientRect), and takes pointer events only over text, so native selection, copy and find work. (4) Trigger: a context-menu row "Select text in image" on img, canvas or video (`crates/wubble/src/menu.rs`), later a hover trigger behind an off-by-default setting. Recognition goes through an `OcrBackend` trait that `ocr-backends` implements. Nothing touches the network.

## Live text in images: Vision backend on macOS, PP-OCR over ONNX Runtime on Linux and Windows (`ocr-backends`)

Key: `ocr-backends` · engine patch: no · state: not written.

TODO line 57, docs/research/ocr.md section 5. Implement the `OcrBackend` trait (from `ocr-pipeline-text-layer`) twice, in a new crate (for example `crates/wubble-ocr`, or a `wubble-ocr` helper process for crash isolation, as the research recommends). macOS: Apple Vision (`VNRecognizeTextRequest`, accurate level, languages from `supportedRecognitionLanguages`) through objc2-vision, returning line text and quads. Linux and Windows: PP-OCRv5 mobile detection, angle classifier and recognition over ONNX Runtime (`ort` crate, MIT/Apache). Either use oar-ocr after confirming its licence, or write a small in-repo pipeline (resize, DB post-processing, CTC decode). Bundle the models (about 25-30 MB) in the release packaging scripts (`scripts/release/`), with a THIRD_PARTY_NOTICES entry per model traced to Paddle's Apache-2.0 release. Keep `auto-download` features off. Run on a low-priority 1-2 thread pool. Windows: the same ONNX backend.

