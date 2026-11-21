# Read aloud: tiny-voca as an on-device voice (shelved)

Researched 2026-11-21 at the owner's request ("see if it solves our screen reader problem"). Status: shelved.
Facts are from the project's GitHub README and Hugging Face card as fetched that day; nothing was run here.

## Answer

tiny-voca is a voice, not a screen reader. A screen reader is the OS's (Orca, VoiceOver, NVDA): it reads
wubble through the accessibility tree wubble already exposes (`wubble-ax`, AccessKit) and speaks with the
OS's own voices. tiny-voca does not replace that and should not: a browser that speaks over the user's
screen reader is a bug.

What it does fit is a **Read aloud** feature (read this page, this selection, or Reader view) with no cloud
voice and no OS speech dependency. Today wubble has none, and Chromium's `speechSynthesis` on Linux depends
on speech-dispatcher being installed.

## The model

| | |
|---|---|
| Source | `github.com/n4ze3m/tiny-voca`, `huggingface.co/n4ze3m/tiny-voca` |
| Licence | Apache-2.0 for code and weights; dataset CC BY 4.0 |
| Size | 4,537,291 parameters; ONNX 21.4 MB (float32); GGUF 8.7 MB (Q4_0) to 14.9 MB (F16); phonemizer data 5.4 MB |
| Output | 24 kHz mono |
| Speed | "about 70x real time on four CPU threads" (CPU not named); time to first audio and memory not stated |
| Voices | Two, American English (F2, M2); no cloning |
| Controls | Speed, pitch shift, pitch range; no SSML |
| Long text | One sentence per call (up to 512 tokens, 32 s), joined; no streaming inside a sentence |
| Phonemes | The author's `dizione`, data derived from the CMU Pronouncing Dictionary; eSpeak NG is not named as a run-time need |
| Run times | PyTorch, ONNX Runtime, onnxruntime-web; community ports: llama.cpp (GGUF), Core ML, two Rust ports, ESP32-S3 |
| Quality | Open TTS Leaderboard 2026-11-20, English without cloning: F2 5th (WER 0.74, UTMOS 4.43), M2 7th; no formal listening test |
| Activity | 46 commits, no tagged release, about 800 stars; forks for Italian, Spanish, Hindi |

Stated limits: English only; "clear but can sound slightly synthetic"; foreign names and unusual spellings
often mispronounced; numbers and abbreviations depend on the phonemizer; the same text is always read the
same way.

## Fit for wubble

- For: tiny (about 15 to 27 MB with phonemizer data), CPU only, far faster than real time, permissive
  licence, sentence-at-a-time suits highlighting the sentence being read.
- Against: English only with two voices, where a browser reads every language; no tagged release;
  pronunciation of names and numbers is weak, and web pages are full of both.
- Not checked: `dizione`'s licence; the CMUdict and homograph data licences; whether the Qwen3-TTS terms
  allow training on its output (the training speech was generated with it); the Rust ports' quality;
  real speed and memory on the owner's machines.

## If picked up

1. Read aloud is a feature on Reader view's text extraction: sentences in, audio out, the spoken sentence
   highlighted; one state machine in `wubble-core` (idle, loading, speaking, paused), effects at the edge.
2. Voice behind one trait with two implementations: the OS voice (speech-dispatcher, AVSpeechSynthesizer,
   SAPI: every language, nothing to ship) and a bundled or opt-in model. Start with the OS voice; a model
   is the fallback where the OS has none (most Linux installs).
3. The model is an opt-in download unless small enough to ship; either way no request by default.
4. Compare at that time with the other small open voices (Piper, Kokoro, KittenTTS) on languages,
   licence (Piper's eSpeak NG phonemizer is GPL-3.0-or-later, which wubble can carry) and size.
5. Never speak when a screen reader is running unless asked.
