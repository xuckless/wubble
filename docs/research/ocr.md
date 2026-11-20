# Live text (on-device OCR in images): options research

Question: Which on-device OCR could power Live Text (select text in images) without anything leaving the machine?
Status: Shelved: Vision on macOS, PP-OCR over ONNX Runtime elsewhere, behind one backend trait, when the feature is picked up.

Date: 2026-11-18. Status: feature SHELVED, this note is for choosing a model when it is picked up.
Method: web search and page fetches today. Every claim is tagged **[V]** verified from a cited source, **[I]** inferred or from my own background knowledge (check before relying), or **[?]** not found. Things that change fast (versions, OS availability) are date-stamped. Sources are numbered at the end.

## 1. Comparison table

Code licence / weights licence must be GPL-3.0-compatible to bundle. "Boxes" = what the engine gives us to lay a selectable text layer.

| Option | Code licence | Weights licence | Download size | CPU speed, 1080p-ish | GPU path | Scripts | Boxes | Rust integration | Verdict |
|---|---|---|---|---|---|---|---|---|---|
| macOS Vision (`VNRecognizeTextRequest`, new `RecognizeTextRequest` macOS 15+) | OS API | OS-owned, nothing bundled | 0 | Apple silicon: fast, ANE/GPU [I] | yes, system [I] | Latin + CJK + more; runtime list via `supportedRecognitionLanguages` [V:1] (full list not confirmed) | line + per-substring boxes, normalised quads [I] | `objc2-vision` raw bindings (feature-gated `VNRecognizeTextRequest`) [V:2]; `apple-vision` wrapper crate exists [V:2]; nobody's example found [?] | **First choice on macOS** |
| Windows `Windows.Media.Ocr` | OS API | OS-owned | 0 (user installs language packs in Settings) [V:3] | OK [I] | no | only installed OCR language packs [V:3] | line + word rects, axis aligned [V:3 partial] | `windows` crate WinRT [I] | Windows baseline, acceptable |
| Windows App SDK `TextRecognizer` | OS API | OS-owned | model installed by `EnsureReadyAsync` | NPU only [V:4] | NPU | not documented | words, lines, **polygon** boundaries, confidence [V:4] | WinRT/C++; needs Windows App SDK runtime [I] | Copilot+ PCs only: a bonus path, not a baseline |
| Linux OS OCR | none | none | | | | | | | nothing built in [I] |
| **PaddleOCR PP-OCRv5/v6 (ONNX)** | Apache-2.0 [V:5] | Apache-2.0 [V:5, V:6 for ONNX conversions] | mobile rec 16 MB, server rec 81 MB, server det 84 MB [V:7]; Latin/Cyrillic/Korean/Thai/Greek mobile recs ~7.5 MB each [V:7]; v6 tiers 1.5M/7.7M/34.5M params [V:5] | Paddle runtime on Xeon: v5 mobile 1.75 s/image, server 4.34 s [V:8] (Paddle, not ORT, doc-sized test images; peak RAM 2.2 GB mobile in that harness [V:8]) | ORT CUDA yes [V:9]; CoreML/DirectML EPs exist in ORT [I] | v5: Simplified/Traditional Chinese, pinyin, English, Japanese [V:8]; v6: 50 languages [V:5]; Arabic/Indic need checking [?] | detection gives polygons (DB), lines; word split needs work [I] | `oar-ocr` 0.11.0 on `ort` rc.15 [V:9]; `paddle-ocr-rs` Apache-2.0 [V:10]; own thin pipeline is feasible [I] | **First choice for Linux and uniform fallback** |
| PaddleOCR-VL 0.9B | Apache-2.0 [V:11] | Apache-2.0 [V:11] | not found [?] | no CPU numbers [?] | CUDA/Metal in `oar-ocr-vl` [V:9] | 109 languages [V:11] | page-level parsing | same crate family | too heavy for hover-time live text |
| `ocrs` (Rust-native, rten) | Apache-2.0 OR MIT [V:12] | **CC-BY-SA 4.0** [V:13] (trained on HierText, CC-BY-SA 4.0 [V:14]) | not stated [?] | not stated; "early preview, more errors than commercial engines" [V:12] | none | **Latin only** [V:12] | words/lines; rotation not mentioned [V:12] | native Rust, best fit technically | Weights share-alike and Latin-only: reject as primary; maybe CJK-less quick path |
| Tesseract 5 (LSTM) | Apache-2.0 [I] | Apache-2.0 tessdata [I] | tessdata_fast ~ few MB per language [I] | fastest on CPU in informal tests (0.1-0.3 s vs 1-5 s) [V:15], best on clean printed text, worst on noisy/rotated/photos [V:15] | no | 100+ languages [I] | word boxes, no rotation per word [I] | crates stale: `leptess` last release Feb 2023, `tesseract-rs` Oct 2024, `rusty-tesseract` shells out [V:16]; C++ build dependency | Fallback only (screenshots, Latin) |
| docTR | Apache-2.0 [V:17] | not stated on repo page [?] | n/a | PyTorch/TF stack | | | | Python | skip (no Rust path, weights unclear) |
| EasyOCR | Apache-2.0 [I] | [I] | | Python/PyTorch | | 80+ | | Python | skip |
| Surya | Apache-2.0 [V:18] | **modified AI Pubs Open RAIL-M: free only for research, personal, startups under $5M** [V:18] | 689M-param VLM [V:18] | 4.5 pages/s on an H100; 0.19 pages/s on M4 Max [V:18] | | 90+ | | Python | **excluded: weights not GPL-compatible** |
| TrOCR / Florence-2 | MIT [I] | MIT [I] | 230M-ish [I] | seconds per crop on CPU [I] | | | recogniser only (TrOCR) / generative (Florence-2) | | skip |
| MMOCR | Apache-2.0 [I] | | | PyTorch | | | | | skip, maintenance mode [I] |
| Chrome Screen AI library | **closed binary** (built in google3, not public) [V:19] | n/a | ~100 MB reported by a Brave user [V:20] | on-device [V:19] | | | VisualAnnotation boxes [I] | none | **not reusable** (see section 3) |

## 2. Per-option detail

### 2.1 OS OCR

macOS. Vision's `RecognizeTextRequest` is the Swift-native struct, macOS 15.0+ [V:1]. The older `VNRecognizeTextRequest` (Objective-C, macOS 10.15+ [I]) is what Rust would call. Two recognition levels, fast and accurate [V:1]; the available languages differ by level and by language correction, so query `supportedRecognitionLanguages` at runtime instead of hard-coding [V:1]. Setting `recognitionLanguages` improves accuracy [V:1]. Results are `VNRecognizedTextObservation` with a normalised bounding box plus per-candidate string, and `boundingBox(for: range)` for sub-line boxes [I]. This is exactly what Safari Live Text and Firefox-on-macOS use (section 3). Rust: `objc2-vision` exposes `VNRecognizeTextRequest` behind a Cargo feature [V:2]; the `apple-vision` crate wraps it for CGImage/CVPixelBuffer input [V:2]. I found no worked example for the completion-handler block with objc2; plan a half-day spike. Using the synchronous `VNImageRequestHandler.performRequests` on a worker thread avoids block wiring [I]. Speed on Apple silicon: no benchmark found [?]; Vision runs on the Neural Engine/GPU and is fast enough for per-image hover use [I]. Text orientation: Vision handles rotated text moderately; boxes are axis-aligned bounding boxes, so for tilted lines the text layer has to be an approximation [I].

Windows. `Windows.Media.Ocr.OcrEngine` is the legacy universal API: engine per language; `AvailableRecognizerLanguages` lists only installed OCR packs, installed by the user in Settings; `TryCreateFromLanguage` returns null if missing [V:3]. Result tree lines → words with rects [V:3 partial]. The new Windows App SDK text recognition (`Microsoft.Windows.AI.Imaging.TextRecognizer`) returns characters, words, lines, polygon boundaries and confidence, but runs **exclusively on NPU devices** (Copilot+ PCs) [V:4] (doc dated 2026-10-01). So Windows needs a baseline that is not the new API: either legacy OCR (needs language packs, which is a Windows-side download the user triggers, fine for the rules) or the cross-platform ONNX model. Windows is CI-only for wubble, so this can wait.

Linux. No OS API [I]. Cross-platform model needed.

### 2.2 Open models

PaddleOCR is the clear winner among open options:
- Latest release PaddleOCR 3.7.0 (2026-06-11), Apache-2.0 [V:5]. PP-OCRv6 (50 languages, tiny 1.5M / small 7.7M / medium 34.5M params) landed in 3.7.0 [V:5]; PP-OCRv5 remains the better-documented generation with ONNX conversions in circulation.
- PP-OCRv5 sizes (Hugging Face ONNX conversion, Apache-2.0 labelled): mobile rec 16 MB, server rec 81 MB, server det 84 MB; per-language mobile recs ~7.5 MB (English 85.25%, Thai, Greek) [V:7]. Mobile detection size not found; ~5 MB is my recollection [I]. A full mobile Latin+CJK set is therefore about 25-30 MB [I].
- Accuracy on Paddle's own benchmark: v5 server det 0.827 / mobile 0.770; server rec 0.840 / mobile 0.802 [V:8]. CPU: v5 mobile 1.75 s per image, server 4.34 s [V:8], v5 slower than v4 because of the larger dictionary [V:8]. Those are Paddle Inference on an Xeon Gold 6271C with document-pipeline preprocessing, so treat them as upper bounds. ONNX Runtime with a capped long side (e.g. 960 px) should be under a second for typical web images [I]; measure on the owner's machines.
- ONNX exports exist: RapidOCR (Apache-2.0 mirror seen [V:6]), and Hugging Face conversions [V:6, V:7]. Upstream documents ONNX conversion [V:5].
- Orientation: PP-LCNet text-line/document orientation classifiers are supported by `oar-ocr` [V:9]. This covers 0/180 degree; detection polygons cover arbitrary angles [I].
- Rust: `oar-ocr` 0.11.0 (2026-11-18 build) pins `ort` 2.0.0-rc.15; has CUDA feature, `download-binaries` default feature, `auto-download` of models optional (must stay off: phone-home) [V:9]; its licence was not shown on the page [?]. `paddle-ocr-rs` 0.6.1 is Apache-2.0 [V:10]. The `ort` crate is still release-candidate (rc.12/rc.13 seen) [V:21]: API churn risk, acceptable if wrapped behind one trait. Alternative runtimes: `tract` (pure Rust ONNX, op coverage for DB/SVTR unverified [?]), `rten` (ocrs's engine, runs ONNX; same caveat) [I].
- Word-level boxes: DB gives text-line (or word-ish) polygons; recognition returns line text. Splitting a line into words for fine selection needs CTC character timesteps (PaddleOCR can return per-char boxes: `return_word_box`) [I]. For a first slice, line-level spans with a `scaleX` fit are enough: selection works at line granularity plus character-level via text layer widths, which is how pdf.js and Safari behave visually.
- GPU: ORT execution providers (CUDA, CoreML, DirectML; WebGPU EP is experimental) [I]. wgpu-based inference (`burn`, `candle`) has no ready PP-OCR port [?]. Recommendation: CPU first; images are small and OCR is cached. Zero-copy to the shared wgpu device is not needed because input is a decoded image, not a frame.
- ONNX Runtime binary size: roughly 10-20 MB shared library on desktop [I]; `ort` downloads prebuilt binaries at build time [V:9] (check this fits the build-from-source/offline CI rule).

Other engines:
- Tesseract: informal tests show it is the fastest on CPU and best on clean printed text, worst on noisy, rotated or handwritten input [V:15]. On SROIE receipts, character error rate docTR 19.7%, PaddleOCR 20.5%, EasyOCR 28.3%, Tesseract 33.6% [V:15] (vendor benchmark, weak evidence). Rust crates stale or shelling out [V:16]. Keep as a Latin-only fallback if ONNX is rejected; I would not.
- `ocrs`: nicest Rust story (pure Rust via rten), but Latin only, "early preview", weights CC-BY-SA 4.0 because trained on HierText [V:12-14]. CC BY-SA 4.0 is generally treated as one-way compatible with GPLv3 [I], but it is outside AGENTS.md's licence list (MIT/BSD/Apache/MPL), so it needs an explicit owner decision. Latin-only already disqualifies it as the uniform fallback.
- Surya excluded on weight licence [V:18]. docTR/EasyOCR/MMOCR: Python stacks, no Rust path.
- VLMs (PaddleOCR-VL 0.9B [V:11], Florence-2, TrOCR): hundreds of MB to GB, seconds per image on CPU [I]; only sensible as an optional high-quality mode on GPU. `oar-ocr-vl` offers CUDA and Metal [V:9].

## 3. How other browsers do it

- **Safari**: Live Text on top of Vision; closed source, OS API [I].
- **Firefox**: Mozilla blog and support article "Copy text from images on Firefox for macOS" exist [V:22]: right-click "Copy Text from Image", works on jpg/png/webp [V:22]. A 2020 Nightly report said it uses the macOS Vision API (`VNRecognizeTextRequestRevision2`) and that Windows/Linux did not have it, with Windows OCR APIs mooted for Windows [V:23]. No evidence of a Windows or Linux release found [V:22]. So Firefox's answer is "OS API on macOS only" which supports our per-platform plan.
- **Chrome**: Screen AI library (OCR + main-content extraction), developed in google3 (not open), on-device, "does not send any data to network or store on disk" while running, built for ChromeOS/Linux/Mac/Windows [V:19]. On desktop it is **downloaded on demand via the component updater** as component "ScreenAI Library" (`libchromescreenai.so`), kept 30 days after last use [V:24]; ~100 MB reported [V:20]. Used for PDF OCR (Chrome 126+ [V:25]) and Google Lens/image features; a "make the text in PDF images interactable" flag was reported [V:26] (status unverified). Debian disables the download and calls it an opaque blob [V:24]. For wubble: closed binary with unknown redistribution terms, fetched from Google's update servers (violates the no-phone-home rule), and relies on Chrome's component updater and mojo service that CEF OSR does not expose [I]. **Not reusable.** The Chromium-side client code (`optical_character_recognizer.h`, `ScreenAIInstallState`) is open but useless without the blob [V:24].
- **Edge, Brave, Arc**: not researched [?]; Brave users report the Screen AI component downloading, so Brave inherits Chrome's [V:20].

## 4. Integration sketch for wubble

Where pixels come from (ranked):
1. **Original image bytes via the engine's own fetch** (CEF URL request on the page's request context, or `Network.getResponseBody`/`Page.getResourceContent` through DevTools): full resolution, works cross-origin (no canvas taint), hashable. The content script only reports `{src/currentSrc, rect, naturalWidth, objectFit...}`. [I]
2. DevTools `Page.captureScreenshot` with `clip` = element rect (+ scale): works for CSS backgrounds, canvas, video frames and `<img>` with filters; costs an extra render and is limited to the visible area. Good for "select text in this video frame". [I]
3. Reading the OSR frame texture: avoid; it means GPU readback, which AGENTS.md forbids on the frame path. [I]

Pipeline: `wubble-core` holds a pure `OcrState` enum (Idle → Fetching → Recognising → Ready/Failed) with a `transition` fn and effects; `wubble-engine` executes. Recognition on a worker thread pool (Vision needs no special thread; ORT on a 1-2 thread pool with low priority), or a small `wubble-ocr` utility process if we want crash isolation from ORT (recommended: ORT/Vision crashes then do not take the browser down; helper binary already exists as pattern) [I]. Cache: LRU in memory keyed by `blake3(bytes)` + engine version + language set; optional on-disk cache for non-private profiles only. Private windows: memory only, dropped on window close; nothing logged.

Presentation: **injected transparent text layer in the page**, not a Slint overlay. Per recognised line a `<span>` absolutely positioned in a container that tracks the image's box (re-measured by ResizeObserver, scroll/transform via `getBoundingClientRect` on animation frames while visible), `color: transparent; user-select: text; white-space: pre`, font size from box height, `transform: scaleX(...)` to fit measured width, `rotate()` when the polygon is tilted, placed in a closed shadow root so page CSS cannot interfere. Native selection, double-click word select, copy, drag, find-in-page and the context menu then work for free (pdf.js and Safari take the same approach) [I]. Cost: the layer sits above the image and steals clicks/drags; set `pointer-events:none` until text is actually under the pointer (hit-test with the lines' rects in a `pointermove` handler and flip to `auto` only over text), keep image links and drag-save working elsewhere. A Slint overlay would need our own hit testing, selection model, copy and find integration plus scroll sync across OSR frames, so it is more work for worse behaviour.

Triggers (in order of build): (a) context menu "Select text in image" on an `<img>` or canvas/video; (b) hover over an image ≥ ~100×40 CSS px for ~400 ms (cancelled on leave, debounced, one job per image); (c) later: automatic for visible images under a pixel budget, off by default (battery, CPU). Per-site "never" rule shares the settings model with other per-site rules. Skip images < small area, animated GIFs, SVG with real text, and `<img>` whose alt text already contains text.

Privacy: OCR never touches the network; model files are local. Models bundled (~25-30 MB for PaddleOCR mobile det+cls+Latin/CJK rec plus the ORT library) is the simplest option and avoids any download story. If extra scripts (Arabic, Indic, Thai, Greek) are offered later, deliver them as an explicit user-requested download per script (~7.5 MB each [V:7]) with a hash check; never auto-download (this also means `oar-ocr`'s `auto-download` feature must stay off [V:9]).

## 5. Recommendation

- **macOS**: Vision (accurate level, language list from `supportedRecognitionLanguages`), via `objc2-vision` or the `apple-vision` crate, behind an `OcrBackend` trait. 0 MB, best quality on the platform, matches Safari/Firefox.
- **Linux (and uniform fallback, also Windows to start)**: PP-OCRv5 mobile detection + angle classifier + recognition through ONNX Runtime (`ort`), either using `oar-ocr` or a thin in-repo pipeline (resize, DB post-processing, CTC decode ~500-800 lines). ~25-30 MB models bundled plus ORT. CPU first.
- **Windows (CI-only for now)**: same ONNX backend as Linux for parity; add `Windows.Media.Ocr` as an optional native backend when someone tests it on real hardware; use the Windows App SDK `TextRecognizer` only as a Copilot+ bonus.
- Do not use: Surya (weights), `ocrs` as primary (Latin only, share-alike weights), Chrome Screen AI (closed, phone-home download), VLMs for hover use.

Expected: Vision a few hundred ms to ~1 s per web image on Apple silicon [I]; PaddleOCR mobile over ORT well under 1 s on a modern Linux desktop for a 1080p image capped to 960 px detection side [I, measure]; model RAM a few hundred MB peak [I] (Paddle's own harness reported 2.2 GB peak but that includes its runtime [V:8]).

Open risks:
1. `ort` still release-candidate and `oar-ocr` licence not confirmed from the page; verify LICENSE files and consider vendoring a small pipeline.
2. PP-OCR word-level boxes and tilted-text fidelity (affects selection feel). Test on screenshots, memes, photos of signs.
3. Language coverage for Arabic/Indic in the PP-OCRv5/v6 bundles not verified; check before promising.
4. Weights licence: the Apache-2.0 label on third-party ONNX conversions should be traced back to the Paddle release; keep a `THIRD_PARTY_NOTICES.md` entry per model.
5. Text-layer coexistence with page pointer events, and RTL/vertical text layout.
6. objc2 block wiring for Vision (spike), ORT build/CI on three platforms and ungoogled/CEF static linking (ORT bundles its own protobuf; separate dynamic lib avoids symbol clashes) [I].
7. Windows legacy OCR depends on user-installed language packs.

Rough effort for a first slice (context-menu trigger, images only, Linux + macOS, cache, DOM text layer, no hover): ~12-16 working days: Vision backend 2-3; ORT PP-OCR pipeline 5-7 (3-4 if `oar-ocr` fits); image fetch + cache + state machine 2-3; DOM layer and context-menu plumbing 3-4. Hover and auto triggers +2-3 days. Windows native backend +2 days when needed.

## Sources

1. https://developer.apple.com/documentation/vision/recognizetextrequest.md , https://developer.apple.com/documentation/vision/recognizetextrequest/supportedrecognitionlanguages.md (macOS 15.0+, languages by level)
2. https://lib.rs/crates/objc2-vision/features , https://docs.rs/apple-vision/0.13.0/apple_vision/
3. https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr.ocrengine.availablerecognizerlanguages?view=winrt-22621
4. https://learn.microsoft.com/windows/ai/apis/text-recognition (dated 2026-10-01; NPU only; polygons)
5. https://github.com/PaddlePaddle/PaddleOCR (3.7.0, 2026-06-11; v6 tiers; Apache-2.0)
6. https://huggingface.co/monkt/paddleocr-onnx ; https://www.nuget.org/packages/rapidocrsharponnx ; https://gitee.com/byteuser/RapidOCR
7. https://huggingface.co/monkt/paddleocr-onnx/commit/7b02d0a30a07ba2b92ad1ff5a8941ae2c633de65 (sizes); https://www.paddleocr.ai/v3.0.3/en/version3.x/algorithm/PP-OCRv5/PP-OCRv5.html
8. https://www.paddleocr.ai/latest/en/version3.x/algorithm/PP-OCRv5/PP-OCRv5.html (accuracy, CPU time, RAM)
9. https://docs.rs/crate/oar-ocr/latest
10. https://lib.rs/crates/paddle-ocr-rs
11. https://huggingface.co/lvyufeng/PaddleOCR-VL-0.9B/blob/main/README.md ; https://arxiv.org/pdf/2510.14528v4
12. https://github.com/robertknight/ocrs
13. https://huggingface.co/robertknight/ocrs
14. https://github.com/robertknight/ocrs-models
15. https://codesota.com/ocr/tesseract-vs-easyocr ; https://imagetotable.ai/de/references/receipt-ocr-benchmark-results ; https://dev.to/felipe_anyformat/best-python-ocr-libraries-179
16. https://docs.rs/crate/leptess/latest ; https://lib.rs/crates/tesseract-rs ; https://www.lib.rs/crates/rusty-tesseract
17. https://github.com/mindee/doctr
18. https://github.com/datalab-to/surya
19. https://chromium.googlesource.com/chromium/src/+/refs/tags/141.0.7390.65/services/screen_ai/README.md
20. https://community.brave.com/t/brave-is-downloading-100-megabytes-every-time-it-starts/586463
21. https://docs.rs/crate/ort/latest
22. https://support.mozilla.org/sv/kb/text-recognition (Copy text from images on Firefox for macOS)
23. https://www.ghacks.net/?p=180316
24. https://chromium.googlesource.com/chromium/src/+show/205191a32c9db91062c5342e901b2da2d67a49e5/chrome/browser/screen_ai/README.md ; https://chromium.googlesource.com/chromium/src/+/HEAD/chrome/browser/component_updater/screen_ai_component_installer.cc ; https://sources.debian.org/patches/chromium/138.0.7204.157-1/disable/screen-ai-blob.patch
25. https://winaero.com/chrome-126-released-with-ocr-in-pdf-and-google-lens-on-desktop
26. https://windowsreport.com/?p=1443973
