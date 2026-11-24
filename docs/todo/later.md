# Later and shelved

Part of the roadmap ([TODO.md](../../TODO.md)). Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

## After the optimization pass (owner, 2026-11-21)
Built as planned in the research, each as its own pass; nothing downloads or asks a server unless the owner turns it on.
- [ ] Page translation, on-device: the pipeline in [docs/research/translate.md](../research/translate.md) (build-out `translate-detect-offer`, `translate-bergamot-backend`, `translate-selection-reader`). First, spikes of about 1.5 days: does Chrome's `Translator` API answer in CEF's windowless mode (`View::eval` of `Translator.availability`; expected not), Bergamot's HTML and alignment API, the model registry and each model's licence (the models repo is archived). Then a `translate` frame channel and walker script (the cosmetic channel's pattern), `wubble_core::translate` (state, `whatlang` detection), a helper process running bergamot-translator; about 17 days in all
- [ ] Live text in images, on-device OCR: the pipeline in [docs/research/ocr.md](../research/ocr.md)

## Features no plan has yet (scout, 2026-11-22)
Checked against the build-out list and the roadmap; the owner picks.
- [ ] Preferred languages: Accept-Language and the page locale (nothing sets them; translation and spell languages need it first)
- [ ] QR code for a page or a link; search an image (an engine's URL template); copy link to highlight (text fragments)
- [ ] Device pickers for WebUSB, Serial, HID and Bluetooth; FedCM and Payment Request (spike each: does CEF's windowless mode forward them)
- [ ] Per-site user agent ("request desktop site"); history grouped by task; sidebar web panels; the shell in other languages

## Shelved ideas
- [ ] Read aloud with an on-device voice (tiny-voca or the OS voice); a voice, not a screen reader ([docs/research/tts.md](../research/tts.md))
- [ ] OS password managers and passkey keychains (fingerprint, passkeys where the laptop has them), as a passthrough in a dedicated build after extensions ([docs/research/apple-passwords.md](../research/apple-passwords.md))
