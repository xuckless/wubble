# Page translation: options research

Question: How could wubble translate pages without phoning home?
Status: Shelved: Bergamot (Firefox Translations) in a helper process with per-pair models the user downloads; Chromium Translate does not fit.

Date: 2026-11-18. Research only, no code. Tags: **[V]** verified from a cited source, **[I]** inferred or from my own background knowledge (check before relying), **[?]** not found. Sources are numbered at the end. Constraint throughout: nothing phones home by default; any network translation is an opt-in, off-by-default backend.

## 1. Comparison table

| Option | Code licence | Model licence | Size per language pair | Speed | Coverage | Offline | Fit for wubble |
|---|---|---|---|---|---|---|---|
| Chromium classic Translate (infobar/bubble) | BSD | n/a (server) | n/a | n/a | Google's | **no**: calls Google, needs API key [V:1,2] | No: chrome/ UI layer, not in CEF OSR [I], needs Google key, stripped by ungoogled [V:3 secondhand] |
| Chromium Translator + Language Detector web APIs (Chrome 138+) | BSD for glue | **closed "Chrome TranslateKit" binary + Google language packs** via component updater [V:4,5] | not documented [?] (third-party claims 1-2 GB [V:4]) | local | 39 entries [V:4] | after first download | No: closed blob fetched from Google; wrong layer for embedders |
| Chromium language detection (CLD3 / TFLite langid) | BSD/Apache | model downloaded via optimization guide for TFLite [V:6,I] | small | fast | many | yes once present | Use something else (see 5) |
| **Firefox Translations / Bergamot** | MPL-2.0 [V:7] | **MPL-2.0** [V:8] | student models ~17 MB (en-pt example), 32 MB tiny en-fr bundle [V:9,10] | native: ~6,500 words/s on a 2016 desktop, ~1,000 on a 2012 laptop; WASM: 500-600 words/s [V:11] | Mozilla's list (dozen at launch in 118, more since; Vietnamese/Trad. Chinese in 148, Norwegian/Thai in 149; Japanese/Korean/Chinese added) [V:12,13] | yes | **First choice** |
| Apple Translation framework (macOS 15+) | system | system | system-managed | fast, ANE [I] | Apple's list | after download | Good macOS bonus; awkward from non-Swift (see 3) |
| Opus-MT (Helsinki-NLP) via Marian/CTranslate2 | MIT repo [V:14] | **CC-BY-4.0** [V:14] | ~75-300 MB fp32, ~40-100 MB int8 [I] | good | 1000+ pairs [I] | yes | Good source for pairs Mozilla lacks (needs attribution) |
| Argos Translate + `ct2rs` | MIT or CC0 [V:15]; `ct2rs` MIT [V:16] | per package, **unverified** [V:15] | ~50-100 MB [I] | OK | ~50 languages, ~100 packages [V:15 third-party] | yes | Backup engine path; package licences to audit |
| NLLB-200 | | **CC-BY-NC** [I, as stated by owner] | | | 200 | | **Unusable** (non-commercial) |
| MADLAD-400 3B | | Apache-2.0 [V:17] | GB-scale int8 (3.1 GB MLX int8 [V:17]) | slow on CPU [I] | 400+ [V:17] | yes | Optional "high quality / rare language" pack on strong machines |
| TranslateGemma 4B/12B/27B | | Gemma terms, **not verified** [?] | multi-GB [I] | slow on CPU | 55 [V:18] | yes | Skip: licence unclear, size |
| LibreTranslate | **AGPL-3.0** [V:19] | Argos models | n/a | n/a | | self-host | Optional external endpoint only |
| DeepL API Free | n/a | n/a | 0 | network | | **no** | Optional off-by-default backend [V:20] |

## 2. What Chromium has

### 2.1 Classic Translate
- Chromium's design: compare page language with the browser's configured language and show an infobar offering translation [V:21 partial]. Chromium translate is **disabled automatically when the build has no Google API key** ("the translate feature is automatically disabled when the API key is not used to compile chromium"; the 2015 change disabled the dialog when no key is configured) [V:1,2]. Distro builds without keys (Fedora, Arch) therefore show no translate prompt [V:22,23]. Even with a key, an Arch report found language detection failing because the TFLite model needed a separate optimization-guide key [V:23].
- Server: Google's translate endpoints (translate.googleapis.com, translate script from Google) [I; the host name is from my own knowledge and was not in any fetched page, verify in `components/translate/core/browser/translate_url_util`].
- CEF: OSR (windowless) always uses **Alloy style**; Chrome style does not support windowless [V:24]. The translate bubble, `ChromeTranslateClient` and infobar plumbing live in `chrome/browser/translate` and `chrome/browser/ui`, which Alloy style does not use [I]. **Conclusion: the classic Translate UI does not exist in wubble's CEF OSR setup, and wiring `components/translate` ourselves means re-implementing the client against Google's servers, which defeats the purpose.** (Specific "does not work in CEF 156 OSR" statement is inference from the style split; confirm with a 10-minute test in cefclient.)
- ungoogled-chromium: sets `kOfferTranslateEnabled` false by default and a "disable translate integration" patch is listed in Cromite's patch list that mirrors ungoogled's [V:3 secondhand]. The owner's future ungoogled build therefore gets no Chromium translate at all and also no component updater downloads (TranslateKit, Screen AI) [I]. Plan on self-contained features.

### 2.2 Translator / Language Detector web APIs (Chrome 138+)
- Shipped in Chrome 138 stable, desktop only [V:4]. Expert models (not Gemini Nano); language packs downloaded on demand; availability states `downloadable`/`available`; ~39 language entries (ar, bg, bn, cs, da, de, el, en, es, fi, fr, he, hi, hr, hu, id, it, ja, kn, ko, lt, mr, nl, no, pl, pt, ro, ru, sk, sl, sv, ta, te, th, tr, uk, vi, zh, zh-Hant); not in workers; cross-origin iframes need `allow="translator"` [V:4]. No embedder, licence or size information in the docs [V:4]; Chrome says no data is sent to Google when using the model, network only for the first download [V:25 partial].
- Delivery: a component "Chrome TranslateKit" (`libtranslatekit.so` / `.dll`, installed under `TranslateKit/lib`) plus one component per language pack (`TranslateKit/models`), through the component updater, gated by feature flags and group policy [V:5,26]. The binary is Google's closed library [I]. An embedder could in principle point the service at a local binary via a command-line path [V:26 partial] but we would still need Google's blob and its licence; not an option.
- So: **not reusable**, and we should not expose these APIs to pages as our own either (could be a later nicety: implement `window.Translator` by shimming to our engine) [I].

### 2.3 Language detection
- Chromium moved CLD2 to CLD3 (2016) [V:27]; `third_party/cld_3` is vendored (Apache-2.0) [V:27]. `components/language_detection` now holds a TFLite-based detector, model fetched at runtime [V:6]. Whether CLD3 is still compiled in current builds was not confirmed [?].
- Firefox uses **CLD2** for page language ID because it already exists in the tree and fastText-in-WASM was slow [V:28].
- For wubble use something that is already Rust and offline: `<html lang>` and `Content-Language` first, then a Rust n-gram detector (`whatlang` MIT, `lingua` Apache-2.0, `whichlang` MIT) run on a text sample [I]. Pick by test accuracy on short text; no network.

## 3. Local engines

### 3.1 Firefox Translations / Bergamot (recommended)
- Engine: `bergamot-translator`, MPL-2.0, Marian-based, compiles native (CMake; intgemm on x86, ruy on ARM) and to WASM [V:7,11,29]. Powers Firefox page translation since Firefox 118 [V:8].
- Models: trained in `mozilla/translations` pipeline; **"The model files are distributed under the MPL 2.0 license"** [V:8], hosted in a public GCS bucket with a model registry as UI and JSON [V:8]. (Earlier add-on era models: I could not confirm any CC-BY-SA terms; only a derivative's training data NTREX is CC BY-SA [V:30].) Check each model's own card before shipping; MPL-2.0 is explicitly allowed by AGENTS.md.
- Size and quality: teacher 798 MB / 192M params / BLEU 52.5 vs quantised student 17 MB / 15.7M params / BLEU 50.7 (en-pt) [V:9]; a tiny en-fr bundle is 32 MB [V:10]; 8-bit int quantisation [V:9]. Non-English pairs pivot through English (two models) [I].
- Speed: native 1,066 words/s on a 2012 dual-core laptop, 6,548 on a 2016 desktop (en-de, 8-bit) [V:11]; ~9,000 words/s on a 2019 desktop per Mozilla's talk [V:11]; WASM builds 150-200 then 500-600 words/s [V:11]. A typical article of 1,500 words: a few seconds native, 3 s+ WASM. No COMET or head-to-head vs Google found [?].
- Languages: ~a dozen at 118 launch [V:12], later Vietnamese and Traditional Chinese (148), Norwegian Bokmål and Thai (149) [V:13]; Japanese/Chinese/Korean added [V:12]; the authoritative current list is Mozilla's translate page [V:13]. New languages arrive without a browser update [V:12]. Exact count 2026 not confirmed [?].
- Firefox's content-side design (all verified at summary level): `TranslationsDocument` translates nodes and attributes near the viewport first, tags nodes in-viewport / beyond / hidden and prioritises accordingly; `<title>` is queued explicitly; a **static** priority queue after the initial pass; MutationObserver handles dynamic content, with open bugs on dedup and a perf regression (bug 1932780) [V:31]; IntersectionObserver prioritisation proposed (bug 1820252) [V:31]. We should copy the algorithm and avoid the known pitfalls (see section 5).
- Rust reuse: no maintained official crate. `marek_translate_locally` (wrapper around translateLocally/bergamot, 1.0.1) and `BergaRust` (an AGPL-3.0 service, not a library) exist [V:32]. So integration means building `bergamot-translator` ourselves and writing a small `cxx`/bindgen shim around its `BlockingService`/`TranslationModel` API [I], run in a utility process (`wubble-translate-helper`) to isolate C++ crashes and CPU use from the browser. A faster-to-prototype alternative is shipping Firefox's WASM build and running it in a hidden CEF page/worker with the same content-script protocol (WASM speed, no C++ build work) [I]. Both can share one `TranslateBackend` trait.
- Model delivery: explicit, user-requested per-pair download (e.g. "Download Spanish → English, 17-35 MB") from the model registry URL given in settings (default empty or the Mozilla bucket only after the user confirms), verified by hash, stored in the profile. No background fetch, no auto-update check. Optionally ship one or two pairs matching the OS locale with the installer so it works out of the box; that is a product decision for the owner.

### 3.2 Other local options
- **Argos Translate**: MIT or CC0 code, CTranslate2 inference, packages `.argosmodel` each bundling CTranslate2 + SentencePiece + Stanza; third-party review counts ~100 packages / ~50 languages [V:15]; package licences not stated, so audit before bundling [V:15]. `ct2rs` (MIT) gives Rust bindings; supports Marian-MT, NLLB, T5, etc.; CPU backends Ruy/MKL/OpenBLAS/oneDNN, CUDA, Apple Accelerate; no Metal, models must be converted [V:16]. LibreTranslate is Argos behind an AGPL-3.0 server [V:19].
- **Opus-MT**: CC-BY-4.0 models (attribution needed), Marian; MIT tooling [V:14]. Usable with bergamot (Marian format, un-quantised so larger/slower) [I] or CTranslate2.
- **NLLB-200**: CC-BY-NC, excluded [I].
- **MADLAD-400 3B**: Apache-2.0 [V:17], 400+ languages, T5-style, multi-GB; CTranslate2 int8 conversions exist [V:17]. Only as an optional rare-language pack on capable hardware. CPU speed likely a few words/s to tens of words/s [I].
- **Small LLMs**: TranslateGemma 4B/12B/27B, 55 languages [V:18], "4B rivals the 12B baseline" per Google; licence under Gemma terms [?] and multi-GB; skip for now.
- **macOS Translation framework**: on-device, models shared with the system Translate app, system asks permission to download languages (UI dialog) [V:33]. `translation-rs` 0.5.2 (Apache-2.0 or MIT) gives safe Rust bindings through a Swift bridge, weak-linked; typed helpers need macOS 14+, `LanguageAvailability` needs macOS 15+, **manual `TranslationSession` construction needs macOS 26+**, translation completes on the main queue so the main run loop must keep running, 10 s/60 s timeouts, and `#[tokio::main]` style executors starve it [V:34]. Because the API is designed around SwiftUI sessions, older macOS versions cannot be driven headlessly [V:33 partial, V:34]. Verdict: useful bonus on macOS 26+, but Bergamot gives one code path on all three OSes.
- **Windows**: no public on-device text translation API for apps found [?] (Windows live translation features are Copilot+ system features, not APIs [I]). Bergamot native build is the Windows story.

## 4. Online services as an optional backend (off by default)

All need an explicit settings toggle, a visible "this sends page text to X" notice at first use, and per-site opt-in; never used in private windows unless the user agrees; keys stored in the keyring (same store as autofill vault if that crate allows).
- **DeepL API Free**: 500,000 characters/month hard cap, endpoint `api-free.deepl.com` (Pro: `api.deepl.com`), request size 128 KiB; Free plan content may be stored per third-party summaries, so show a warning [V:20,35]. Key required.
- **Google Cloud Translation**: API key + billing project [I]; the classic Chrome path (translate.googleapis.com) is not licensed for third parties [I].
- **LibreTranslate self-hosted**: AGPL-3.0, Argos-powered, no third party if the user runs it; may or may not need an API key depending on setup [V:19]. User supplies base URL.
- **Lingva**: unofficial scraping front-end of Google Translate, ToS-grey [I]; not recommended.
Implementation: a `TranslateBackend` trait with `Local(Bergamot)` and `Remote{endpoint,key}`; the content-script protocol is identical.

## 5. Integration sketch

1. **Detection**: on `DOMContentLoaded`, content script reports `<html lang>`, `Content-Language`, and a sample of ~2 KB visible text; core runs `detect(sample)` (Rust crate). Offer only when detected ≠ user's preferred languages and a model pair exists or is downloadable.
2. **Offer**: the address pill shows a "Translate page" chip; clicking opens a small popover (source/target selectors, "always translate this language", "never translate this site", "never translate this language"). Rules live in the settings store (data table: per-site and per-language, always/never/ask). One `TranslateState` enum (Idle → Offered → Downloading(model) → Translating(progress) → Translated → Failed) and a pure `transition`, effects executed by the engine.
3. **Translating in place**: the content script walks the DOM in viewport order (as Firefox does [V:31]), groups text into **translation units**: nearest block ancestors whose children are text and inline elements (a, b, i, em, strong, span, code-less). Each unit is sent as HTML with inline tags kept (Bergamot has an HTML mode that keeps tags aligned [I, from Firefox usage; verify in the API]) in **batches of ~20-50 units**, replies are written back text-node by text-node using the alignment, keeping the original in a `WeakMap<Node, string>`. Attributes: `title`, `alt`, `placeholder`, `aria-label`. **Skip**: `script`, `style`, `noscript`, `code`, `pre`, `kbd`, `samp`, `textarea`, `input` values, `contenteditable`, `[translate=no]`, `.notranslate`, `math`, `svg text` initially, elements already in the target language (`lang` match).
4. **Live content**: one `MutationObserver` with rAF-coalesced queue, dedup to root-most changed nodes, ignore our own writes by tracking a "translated" marker; throttle SPA churn (Firefox's open bugs show Twitter/X queue stalls and a MutationObserver perf regression [V:31]). New nodes join the viewport-priority queue; Intersection-based reprioritisation can be added (Firefox keeps its queue static [V:31]).
5. **Show original**: restore from the WeakMap and disconnect the observer; toggle in the pill (original ⇄ translated) keeps models loaded and results cached per `(unit-hash, pair)` so toggling back is instant.
6. **Selection translation**: context menu "Translate selection" sends the selection text through the same backend; result in a Slint popover with copy and "replace with"-less read-only view. Uses a different (non-DOM) path, so no page mutation.
7. **Reader mode**: translate the extracted article through the same unit pipeline inside the reader document; do not translate before extraction (extraction is structure-based and would see translated text only if run after). Record the last choice per site.
8. **Engine process**: `wubble-translate-helper` (utility process, child of the browser, no network) owns models, accepts `{pair, units[]}` over a pipe, returns translated units. The browser process never links C++ translation code.
9. **Privacy**: all in-process/local; remote backends clearly labelled, off by default, not active in private windows without a per-window confirm. No telemetry. Language detection samples never leave the device.

## 6. Recommendation and first slice

- **Primary**: Bergamot (native, MPL-2.0 engine and models) in a helper process, models downloaded per pair on explicit user action. One engine on Linux, macOS, Windows. Reasons: licences are clear, sizes small (tens of MB per direction), CPU speed adequate for pages (seconds), Mozilla already validated the DOM approach, no phone-home.
- **Fallbacks**: (a) Opus-MT/Argos via `ct2rs` for pairs Mozilla lacks (CC-BY-4.0 attribution; audit each package licence); (b) macOS 26+ Translation framework through `translation-rs` as an optional backend; (c) optional DeepL Free / self-hosted LibreTranslate remote backend, off by default; (d) MADLAD-400 as an advanced rare-language pack.
- **Do not use**: Chromium Translate (Google server + key, absent in OSR, stripped by ungoogled), Chrome Translator API (closed TranslateKit blob, component updater), NLLB-200 (non-commercial), TranslateGemma until its licence is read.

First slice (Linux + macOS, en ⇄ one or two pairs, desktop pages): effort **~14-18 working days**:
- bergamot-translator native build + `cxx` shim + helper process: 5-6 days (CMake/intgemm vs ruy and CI on three OSes is the risk; to start sooner, run Firefox's WASM Bergamot in a hidden page, 3-4 days, at a 10x speed cost, and swap later).
- Model download manager with hashes and settings UI (explicit, per pair): 2-3 days.
- Content-script walker, batching, in-place writeback with inline markup, show-original, MutationObserver: 4-5 days.
- Language detection (crate), offer pill, per-site/per-language rules, state machine: 3 days.
- Selection translation + reader hookup: +2 days (can follow).
Risks: HTML/tag alignment in Bergamot API, SPA mutation churn, CJK segmentation, model licence trace per pair, ungoogled/CEF link of C++ (separate process avoids symbol conflicts), RTL layout after translation.

## Sources

1. https://chromium.googlesource.com/chromium/src/+/4baa4206fac22a91b3c76a429143fc061017f318 (translate disabled when no API key; `--disable-translate` removed)
2. https://chromium.googlesource.com/chromium/src/+/8788f3c1260e54476d13d29acc02cf1412bb73f0 ("Disable translate when there is no API key")
3. https://www.mintlify.com/uazo/cromite/reference/patch-list ; https://deepwiki.com/ungoogled-software/ungoogled-chromium/2.1-privacy-features
4. https://developer.chrome.com/docs/ai/translator-api ; https://developer.chrome.com/docs/ai/translate-on-device ; https://techhub.iodigital.com/articles/built-in-ai-in-chrome
5. https://chromium.googlesource.com/chromium/src/+/lkgr/chrome/browser/on_device_translation/component_manager.cc ; https://sources.debian.org/src/chromium/139.0.7258.127-1/sandbox/policy/mac/on_device_translation.sb
6. https://sources.debian.org/src/chromium/140.0.7339.127-1/components/language_detection/README
7. https://github.com/mozilla/translations (MPL-2.0); https://asuth.searchfox.org/mozilla-central/source/toolkit/components/translations/bergamot-translator/moz.yaml
8. https://github.com/mozilla/translations (model files MPL-2.0; GCS bucket; Firefox 118+)
9. https://hacks.mozilla.org/2022/06/training-efficient-neural-network-models-for-firefox-translations/
10. https://huggingface.co/fama-corp/offline_translate (en-fr tiny 32.3 MB bundle)
11. https://ar5iv.labs.arxiv.org/html/2109.10194 (TranslateLocally) ; https://w3.org/2020/06/machine-learning-workshop/talks/privacy_focused_machine_translation_in_firefox.html ; https://github.com/mozilla/bergamot-translator/releases
12. https://www.ghacks.net/?p=203538 ; https://chipp.in/news/firefox-translate-is-finally-getting-support-for-japanese-chinese-and-korean/ ; https://chipp.in/news/mozilla-adds-new-languages-to-firefox-translations/
13. https://connect.mozilla.org/t5/user/viewprofilepage/user-id/53427 ; https://support.mozilla.org/en-us/kb/website-translation ; https://www.firefox.com/features/translate/
14. https://huggingface.co/Helsinki-NLP/opus-mt-en-de/blob/main/README.md?code=true ; https://github.com/Helsinki-NLP/Opus-MT
15. https://cdn.jsdelivr.net/gh/argosopentech/argos-translate@master/README.md ; https://www.promptquorum.com/power-local-llm/argos-translate-review
16. https://docs.rs/crate/ct2rs/latest (0.11.2, MIT)
17. https://huggingface.co/jbochi/madlad400-3b-mt ; https://huggingface.co/Heng666/madlad400-3b-mt-ct2-int8 ; https://soniqo.audio/guides/translate
18. https://blog.google/innovation-and-ai/technology/developers-tools/translategemma/ ; https://www.infoq.com/news/2026/01/google-translategemma-models
19. https://github.com/LibreTranslate/LibreTranslate (AGPL-3.0)
20. https://support.deepl.com/hc/en-us/articles/360021200939-DeepL-API-Free ; https://developers.deepl.com/docs/resources/usage-limits.md
21. https://www.chromium.org/developers/design-documents/translate/
22. https://bugzilla.redhat.com/show_bug.cgi?id=2209275
23. https://bugs.archlinux.org/task/76268
24. https://cef-builds.spotifycdn.com/docs/126.2/cef__types__runtime_8h.html ; https://groups.google.com/g/cef-announce/c/s1WaovAopFo (windowless always uses Alloy style)
25. https://developer.chrome.com/docs/ai/language-detection
26. https://chromium.googlesource.com/chromium/src/+/HEAD/chrome/browser/component_updater/translate_kit_component_installer.cc
27. https://chromium.googlesource.com/external/github.com/google/cld3/ ; https://codereview.chromium.org/2243433002
28. https://firefox-source-docs.mozilla.org/toolkit/components/translations/resources/02_contributing.html ; https://bugzilla.mozilla.org/show_bug.cgi?id=1836974 ; https://bugzilla.mozilla.org/show_bug.cgi?id=1904628
29. https://github.com/mozilla/bergamot-training
30. https://huggingface.co/ordicio/translator
31. https://arai.searchfox.org/firefox-main/source/toolkit/components/translations/content/translations-document.sys.mjs ; https://bugzilla.mozilla.org/show_bug.cgi?id=1932780 ; https://bugzilla.mozilla.org/show_bug.cgi?id=1820252 ; https://bugzilla.mozilla.org/show_bug.cgi?id=1834092 ; https://bugzilla.mozilla.org/show_bug.cgi?id=1840705
32. https://lib.rs/crates/marek_translate_locally ; https://github.com/zh-ww/BergaRust
33. https://blakecrosley.com/blog/apple-translation-framework-on-device ; https://developer.apple.com/documentation/translation/translationsession/init(installedsource:target:preferredstrategy:).md
34. https://docs.rs/crate/translation-rs/latest (0.5.2)
35. https://www.eesel.ai/blog/deepl-pricing
