# Mog: Requirements and Recommendations

*A floating desktop companion for Databricks Genie: hold a key, speak, and get cleaned text back. Turn a setting on and the same utterance can go to Genie as a data question.*

Research cutoff: 2026-11-14. Fact-check corrections have been applied. Items marked **[UNVERIFIED]** need a spike or a confirmation from the Databricks account team before you commit to them.

---

## 1. TL;DR: recommended stack

1. **Desktop shell: Tauri v2 with a React/TypeScript UI and a Rust "Mog Core".** Mog Core owns the hotkeys, audio, speech-to-text, text insertion, the keychain and the HTTP transport, and it exposes a C ABI.
   - *Fallback:* a Flutter shell calling the same core through `dart:ffi`. Choose this if mobile becomes the primary product.
   - *Why not Flutter first, given you prefer it:* push-to-talk needs to see key-up from any app. `hotkey_manager` 0.4 delivers key-up on macOS and Wayland only. There is also no maintained Flutter NSPanel plugin, and Flutter's multi-window API is still experimental.
   - The global hotkey, the floating overlay, pasting into other apps and system-audio capture cannot be ported to iOS or the web on any stack, so Flutter's portability gain is limited to the transcript and Genie UI.
   - Handy (MIT, about 38.7k stars) already ships the hardest 70% on Tauri: push-to-talk, Silero VAD, Parakeet and an overlay.
2. **Hotkey:** your own native hook. On macOS that is an active `CGEventTap`, which needs only the Accessibility permission. On Windows it is `WH_KEYBOARD_LL`. The interaction follows Wispr Flow: hold to talk, double-tap for hands-free, Esc to cancel. The Tauri `global-shortcut` plugin is the fallback for chord shortcuts.
3. **Speech-to-text for push-to-talk: local NVIDIA Parakeet TDT 0.6B v3 (int8 ONNX) on sherpa-onnx (v1.13+) with Silero VAD v6.2.x.**
   - Expected latency is 0.1–0.7 s after key-up on CPU and about 0.05–0.2 s on the Apple Neural Engine (FluidAudio, added in v1).
   - It costs nothing per use, works offline, and adds punctuation itself.
4. **Speech-to-text in the cloud (when local is not allowed or not available):** pay-per-token Gemini audio input through Unity Gateway (`databricks-gemini-3-1-flash-lite`, `audio_url` content in Chat Completions). It is governed and needs no GPU.
   - It requires cross-geo routing and is not benchmarked yet **[UNVERIFIED latency and verbatim fidelity]**.
   - *Fallback:* an always-on `whisper-large-v3-turbo` endpoint on Custom LLM Serving. That feature is **Public Preview** and needs an admin toggle. An A10G runs about $1,022/month.
5. **Cleanup model (fast path):** `databricks-meta-llama-3-1-8b-instruct` with no reasoning, temperature 0, streamed. It runs after a deterministic filler and dictionary pre-pass and is skipped for utterances under 4 words.
   - *Contenders for the eval:* `databricks-gpt-6-luna` (`reasoning_effort=none`, priority tier) and Gemini 3.1 Flash-Lite (`reasoning_effort=minimal`).
   - *Offline tier:* Apple Foundation Models on macOS 26.
6. **Genie-mode question shaping:** non-streaming Chat Completions with a flat `json_schema`, so Unity Gateway output guardrails can apply.
   - The proposed model is GPT-6 Luna (`reasoning_effort=none`), pending the eval.
   - *Fallback:* `databricks-claude-haiku-5-5` with thinking disabled. It probably goes through the Anthropic Messages API, which Unity Gateway guardrails do not evaluate.
7. **Genie: the GA Chat-mode REST API** (`/api/2.0/genie/spaces/{space_id}/start-conversation` plus polling) behind a `GenieProvider` interface.
   - *Upgrade:* the Genie One API (Beta, SSE) for "ask all my data", enabled only after a probe confirms it.
   - Do not use MCP, the old Beta `/api/2.0/mcp/genie` endpoint (end of life 2026-11-30), or iframe embedding.
   - Treat Genie as **asynchronous**. Typical turns take 10–60 s.
8. **Auth:** OAuth U2M (authorization code + PKCE S256) through the system browser to a loopback redirect, against a per-account **public custom OAuth app** with scopes `all-apis offline_access`.
   - Custom apps can only register `all-apis`, `sql`, `offline_access`, `openid`, `profile` and `email`.
   - Refresh tokens are stored in the OS keychain through the Rust `keyring` crate and are never handed to the WebView.
9. **Meeting mode:** separate mic ("You") and system-audio ("Others") tracks. Audio comes from Core Audio process taps on macOS (14.4+ supported) or WASAPI loopback on Windows. Recordings are crash-safe Ogg Opus segments, transcribed live in chunks of 30 s or less.
   - **Stopping is signal-driven:** the call app releases the mic, the calendar end passes, 15 minutes of silence, or the machine sleeps. There is a 90-minute soft cap and a 4-hour absolute ceiling (Section 5).
10. **Governance:** every call carries the user's identity.
    - Cleanup (and cloud speech-to-text) goes through Unity Gateway model services, which provide per-user rate limits, budgets, the `Databricks-Ai-Gateway-Request-Tags` header and inference tables.
    - Genie is never sent anything automatically by default. Transcripts are stored locally and encrypted.
    - Deployment is a notarized DMG/PKG on macOS and an MSI signed with Azure Artifact Signing on Windows, with MDM policy keys.

---

## 2. Reference architecture

### 2.1 Component diagram

```
+--------------------------------- Mog desktop app (Tauri v2) ----------------------------------+
|                                                                                               |
|  UI layer (React/TS in system WebView)          Mog Core (Rust; C ABI so a Flutter shell can   |
|  +------------------------------+               reuse it via dart:ffi)                         |
|  | Mascot panel  (non-activating|<--events---+  +------------------------------------------+  |
|  |  NSPanel / WS_EX_NOACTIVATE, |            |  | Hotkey: CGEventTap(active)+watchdog |      |  |
|  |  click-through except sprite)|            |  |         WH_KEYBOARD_LL (Windows)     |      |  |
|  | Card panel (key-capable):    |            +--| Audio:  cpal mic 16k mono            |      |  |
|  |  transcript, Genie answer,   |               |         Core Audio process tap /     |      |  |
|  |  settings, onboarding        |               |         WASAPI loopback (meeting)    |      |  |
|  +------------------------------+               |         Silero VAD v6.2.x, AEC (v1)  |      |  |
|  TS domain services:                            | STT:    sherpa-onnx Parakeet v3 int8 |      |  |
|   SttProvider  (local | dbx-gemini | dbx-whisper)|         (+FluidAudio CoreML on Mac)  |      |  |
|   CleanupProvider (prepass | dbx SLM | AppleFM) | Insert: AX/UIA -> clipboard+Cmd/Ctrl+V|      |  |
|   GenieProvider (chat | genie-one | agent)      |         -> copy card                 |      |  |
|   PolicyStore (MDM keys), StateMachine          | Journal: Ogg Opus 5-min segments     |      |  |
|          |                                      | Auth:  PKCE loopback, keyring store  |      |  |
|          +---- invoke('dbx_fetch'/'dbx_sse') -->| HTTP:  reqwest + SSE; injects token, |      |  |
|                                                 |        backoff, 429/Retry-After      |      |  |
|                                                 +------------------+-------------------+      |  |
+-------------------------------------------------------------------|---------------------------+
                                                                    | HTTPS (user U2M token,
                                                                    |  scope all-apis)
+-------------------------------- Databricks workspace -------------v---------------------------+
|  /oidc/v1/authorize, /oidc/v1/token  (custom public OAuth app, per account)                   |
|  Unity Gateway model services (UC securables, e.g. corp.ai.mog_cleanup):                       |
|     FMAPI pay-per-token: llama-3-1-8b-instruct | gpt-6-luna | claude-haiku-5-5 |               |
|                          gemini-3-1-flash-lite (audio_url input; cross-geo required)           |
|  Custom LLM Serving (Public Preview): whisper-large-v3-turbo, GPU_MEDIUM, always-on (optional) |
|  Genie Chat mode (GA):  /api/2.0/genie/spaces/{space_id}/...                                   |
|  Genie One API (Beta):  /api/2.0/genie/one/responses (SSE)                                     |
|  Genie Agent mode (Beta): /api/2.0/genie/agents/{agent_id}/responses (SSE)                     |
|  Serverless SQL warehouse  |  Unity Catalog ACLs, row filters, column masks                    |
|  Observability: system.access.audit (aibiGenie), system.ai_gateway.usage,                      |
|                 OTLP /api/2.0/otel/v1/traces (content-free spans)                              |
+-----------------------------------------------------------------------------------------------+
```

Design notes:
- **Tokens stay in Rust.** The WebView calls `dbx_fetch` / `dbx_sse` commands and the core attaches the bearer token. This keeps tokens out of reach of any XSS in the WebView and avoids browser CORS.
- **Mobile and web builds** reuse the TS providers with a different transport: native on mobile, a Databricks App with on-behalf-of-user (OBO) auth on the web, because CORS for direct browser calls is **[UNVERIFIED]**.
- **Mog Core has a C ABI**, so choosing the shell can be revisited later without rewriting the native work.

### 2.2 Flow A: push-to-talk dictation

```
key-down ─► core opens mic (fast-start unit), mascot=listening, earcon
          ─► 16k frames ─► Silero VAD ─► closed segments pre-decoded by Parakeet while user talks
key-up   ─► decode tail segment ─► raw text shown in pill
          ─► prepass (um/uh/hmm regex, dictionary, numerals) ─► <4 words? skip SLM
          ─► CleanupProvider (stream; deadline 1.2 s) ─► number/entity preservation check
          ─► insert into window focused at key-down: AX/UIA set-selected-text → verify
             → else clipboard + Cmd/Ctrl+V + restore (transient clipboard flags) → else copy card
          ─► history (local, encrypted) ; mascot=idle
```
- If cleanup misses the deadline or drops a number or entity, Mog inserts the raw text instead. The card shows "Show raw" and "Paste last".

### 2.3 Flow B: Genie mode (setting on, or Shift+push-to-talk)

```
utterance ─► ASR ─► Genie-mode SLM (json_schema, non-streaming):
             {cleaned_text, intent, genie_question, entities[], ambiguities[], confidence}
          ─► intent==data_question && confidence≥0.7 ?
                 yes ─► preview toast "Asking <Genie Agent>: …" (1.5 s cancel window)
                 no  ─► "Ask Genie" chip on card (one tap)
          ─► GenieProvider.chat:
             POST /api/2.0/genie/spaces/{space_id}/start-conversation {content, enable_visualization:true}
             poll GET …/conversations/{cid}/messages/{mid}  (1 s → backoff, cap 60 s, give up 10 min)
             status chips: FETCHING_METADATA → ASKING_AI → PENDING_WAREHOUSE → EXECUTING_QUERY
             GET …/attachments/{aid}/query-result   (first N rows, row_count, is_truncated)
             viz PNG via GA download-visualization endpoint (SDK: downloadMessageAttachmentVisualization)
          ─► card: answer text, table, chart PNG, collapsible SQL/thoughts, follow-up chips,
             thumbs up/down, "Open in Databricks"; FOLLOW_UP_QUESTION ⇒ mascot=needs-input
          ─► OS notification + mascot=waving when done
```
- Mog keeps one question in flight per conversation, because Genie returns 409 while a response is active. It runs one Genie conversation per Mog session.
- Context goes inline in the message (`Context: … Question: …`). No endpoint accepts per-request instructions.

### 2.4 Flow C: meeting mode

```
user starts (or accepts "Zoom call detected — record?") ─► consent attestation ─► red badge + timer
  mic track ─┐                                        ┌─► Ogg Opus 5-min segments (flush ~1 s) + journal
  sys track ─┴─► resample 16k ─► AEC(mic, ref=sys) ─► VAD ─┤
                                                      └─► ≤30 s chunks ─► SttProvider (local default)
                                                           ─► rolling transcript (You / Others)
  every 3–5 min ─► cheap SLM chunk notes (timestamped)
stop signal (Section 5) ─► finalize segment ─► optional final re-decode (on AC power)
  ─► reduce pass (strong long-context model) ─► summary, decisions, action items,
     open questions, candidate Genie questions (checkboxes, unchecked)
  ─► user ticks questions ─► sent one at a time on a NEW Genie conversation
  ─► raw audio deleted after transcript commit (default; admin-lockable)
```

### 2.5 End-to-end latency budget

| Stage | Target (p50 / p95) | Evidence and notes |
|---|---|---|
| Key-down to mic live | < 50 ms / 150 ms | The mic opens on key-down, so there is a 100–200 ms risk of clipping the first word. Pre-roll is used only in hands-free and meeting modes. |
| Key-up to final VAD flush | ~30 ms | Earlier segments are already decoded while the user speaks. |
| ASR, local Parakeet v3 int8 (CPU) | 0.2 / 0.7 s | Third-party measurements: 660–806 ms warm for a 3.9 s clip on an i5-10300H; 0.21 s for 5.2 s on a Ryzen 7800X3D. |
| ASR, local, FluidAudio on ANE | 0.05 / 0.2 s | 0.19 s on an M4 in a community benchmark. |
| ASR, cloud Gemini audio via Unity Gateway | ? | **[UNVERIFIED]** Benchmark a 5–10 s clip from the users' region. |
| ASR, warm Whisper-turbo on A10G | 0.4 / 1.0 s (estimate) | Not benchmarked. A cold scale-to-zero call took 95 s in one test. Never use scale-to-zero for interactive work. |
| Deterministic pre-pass | < 5 ms | Regex and dictionary only. |
| SLM cleanup (FMAPI pay-per-token) | 0.3 / 1.0 s (estimate) | No published Databricks TTFT. The client enforces a 1.2 s deadline. Apple Foundation Models measured about 0.9 s median. |
| Insertion | < 50 ms | AX/UIA first, then clipboard. |
| **Dictation total (key-up to text)** | **≤ 1.0 s local+skip / ≤ 1.5 s with cloud cleanup (p95)** | Wispr Flow's reference: under 700 ms p99 on dedicated serving. |
| Genie question shaping (SLM, JSON) | 0.5 / 1.5 s (estimate) | Non-streaming. |
| SQL warehouse wake | serverless 2–6 s; pro/classic about 4 min | Onboarding warns if the space uses a non-serverless warehouse. |
| Genie Chat-mode turn | 5–60+ s | Practitioner report; GSK production averaged 34 s, or 22 s with a cache in front. |
| Genie One API turn | about 54 s average, 82 s max (one MCP-path study) | Async only. |
| Meeting stop to summary ready | < 10 s for the draft, < 60 s for the final (estimate) | Chunk notes are precomputed; one reduce pass over about 12–16k tokens for 60 minutes. |

---

## 3. Requirements checklist

Legend: **M** = MUST, **S** = SHOULD, **C** = COULD.

### 3.1 Functional
| ID | Requirement | Pri |
|---|---|---|
| F1 | Push-to-talk dictation: hold to record, release to transcribe, clean up and insert | M |
| F2 | Cleanup levels: verbatim (no LLM), **light (default)**, heavy, genie | M |
| F3 | "Paste last", "Copy last", and a raw-vs-cleaned toggle on every result | M |
| F4 | Genie mode: a persistent setting plus a per-utterance Ask-Genie modifier; the active Genie Agent name is shown on the mini | M |
| F5 | Meeting mode: record, transcribe live, summarize, and offer checkbox-selected Genie questions | M |
| F6 | Local searchable history with retention options: keep / auto-delete after 24 h / never store; per-item delete | M |
| F7 | Custom vocabulary: user dictionary plus a glossary pulled from the Genie space and UC (`serialized_space`, information_schema), top-K terms injected per utterance (≤ 300 tokens) | S |
| F8 | Offline degraded mode: local STT, pre-pass, on-device SLM where available, otherwise raw text; Genie questions queued as "will ask when online" | S |
| F9 | Multi-workspace profiles (host, client_id, default space, warehouse, model service names), switchable from the tray | S |
| F10 | Language capability matrix with auto-routing (for example, non-European languages go to the Whisper fallback, plus a warning that Genie may be English-tuned) | C |

### 3.2 Desktop shell and OS integration
| ID | Requirement | Pri |
|---|---|---|
| D1 | Global hotkey that reports **key-down and key-up** while another app has focus, including modifier-only keys; active `CGEventTap` with a watchdog that re-enables it (macOS), `WH_KEYBOARD_LL` (Windows) | M |
| D2 | Mascot window that never takes focus: macOS NSPanel `nonactivatingPanel`, `canBecomeKey=false`, Status level, `canJoinAllSpaces\|stationary\|fullScreenAuxiliary`, `hidesOnDeactivate=false`, `ActivationPolicy::Accessory` (via `tauri-nspanel` 2.1); Windows `WS_EX_TOOLWINDOW\|TOPMOST\|LAYERED\|NOACTIVATE` with `showInactive` | M |
| D3 | Two surfaces: a mascot/HUD that never becomes key, and a separate key-capable card panel for typing and Genie | M |
| D4 | Transparent window with click-through on transparent areas (Rust cursor poll toggling `set_ignore_cursor_events`); hit regions re-applied on display or DPI changes; topmost re-asserted by a self-healing watchdog | M |
| D5 | Tiered text insertion: macOS AX `kAXSelectedTextAttribute` (verify by re-reading) / Windows UIA, then clipboard + synthesized Cmd/Ctrl+V + restore, then a copy card. Never insert into password fields or while secure input (`IsSecureEventInputEnabled`) is active. Terminals default to clipboard paste. | M |
| D6 | Transient clipboard writes marked `org.nspasteboard.TransientType`/`ConcealedType` (macOS) and `ExcludeClipboardContentFromMonitorProcessing` / `CanIncludeInClipboardHistory=0` (Windows), so transcripts stay out of clipboard history and sync | S |
| D7 | Tray/menu-bar icon, launch at login, single instance, loopback OAuth listener | M |
| D8 | First-run onboarding that checks Microphone, Accessibility and System Audio Recording, and detects revoked permissions at runtime | M |
| D9 | Hotkey conflict detection: read `AppleFnUsageType`, probe whether the shortcut can be registered, warn when Wispr, Superwhisper, ChatGPT/Codex or Raycast are installed | S |
| D10 | Reset a stuck recording on lock, sleep or fast user switching; resync physical key state | M |
| D11 | Accessibility: every mascot action also reachable from the tray menu and the card panel; NSAccessibility/UIA announcements on state changes; reduced-motion poster frame; never signal state with color alone | M |
| D12 | Resource budget: under 150 MB RAM and under 1% CPU at idle without a model loaded; unload the STT model after 15 minutes idle | S |

### 3.3 Audio
| ID | Requirement | Pri |
|---|---|---|
| A1 | Mic capture at the device rate, resampled to 16 kHz mono float32 | M |
| A2 | Silero VAD **v6.2.x** (512-sample windows at 16 kHz) in every mode | M |
| A3 | Mic opens on key-down for push-to-talk; no always-open pre-roll except in hands-free and meeting modes | M |
| A4 | Start and stop earcons; the waveform appears only once capture is confirmed | M |
| A5 | Survive device and route changes (Bluetooth HFP switch, headset plug-in) without losing segments | M |
| A6 | Detect sustained all-zero buffers (a silently denied TCC permission) and show a fix-it UI | M |

### 3.4 ASR
| ID | Requirement | Pri |
|---|---|---|
| R1 | `SttProvider` interface with implementations: `local-sherpa` (Parakeet v3 int8), `local-fluidaudio` (Mac), `dbx-gemini-audio`, `dbx-whisper` (OpenAI-compatible `audio.transcriptions`) | M |
| R2 | Model resident in memory (reloading costs 3–6 s); threads set to the number of **physical** cores | M |
| R3 | Model downloaded on first run from a CDN with checksum and resume (Parakeet int8 about 640 MB); license attribution in About | M |
| R4 | Cloud payloads: chunks of 30 s or less as 16 kHz FLAC/Opus; stay under 4 MB (FMAPI/external/agent) and 16 MB (custom endpoints); never approach the fixed 597 s execution limit | M |
| R5 | Never rely on scale-to-zero for interactive ASR | M |
| R6 | Whisper/Gemini guards: VAD-trimmed input, empty segments dropped, hallucination filter (no-speech and compression-ratio thresholds; known phrases such as "Thank you for watching") | M |
| R7 | Hotword/vocabulary biasing (sherpa-onnx transducer hotwords; Whisper `initial_prompt`) **[UNVERIFIED for Parakeet TDT]** | C |
| R8 | Opt-in multilingual fallback download: whisper.cpp large-v3-turbo (sherpa-onnx Whisper support, or `whisper_ggml` 2.9.0 on Flutter) | S |

### 3.5 SLM cleanup
| ID | Requirement | Pri |
|---|---|---|
| L1 | No reasoning on the fast path: non-reasoning models, `reasoning_effort=none/minimal`, or thinking disabled | M |
| L2 | "Transcription enhancer, not a chatbot" system prompt (VoiceInk pattern) with few-shot examples of correct and incorrect output: never answer, last stated value wins, same language, output only the text | M |
| L3 | Transcript, clipboard and window context passed as delimited data tags; commands inside them are ignored (prompt-injection hygiene) | M |
| L4 | `max_tokens ≈ 1.5 × input + 50` (FMAPI reserves `max_tokens` against output tokens per minute); 429 retry with jitter; fallback endpoint chain | M |
| L5 | Model and endpoint names are config values, never hard-coded (DeepSeek V4.1 Flash retires 2026-11-19; Gemini 3.6 Flash is redirected to 3.8 Flash, which rejects `minimal`; GLM 5.2 retires 2027-02-04) | M |
| L6 | Deterministic check that every number and glossary entity survives cleanup, otherwise fall back to raw text | M |
| L7 | Flat `json_schema` for Genie mode (no `anyOf`/`oneOf`/`allOf`/`$ref`/`pattern`; at most 64 keys); plain dictation path uses no JSON | M |
| L8 | Static prompt prefix first to benefit from prompt caching where supported **[UNVERIFIED for Llama/Gemma/Haiku]** | C |
| L9 | Golden set of 150–300 utterances plus an MLflow GenAI eval measuring the answered-instead-of-cleaned rate, entity/number preservation, edit distance, and p50/p95 latency measured from the desktop | M |
| L10 | On-device SLM tier: Apple Foundation Models (macOS 26 with Apple Intelligence; 4096-token context; fresh session per call; prewarm during recording) | S |

### 3.6 Genie integration
| ID | Requirement | Pri |
|---|---|---|
| G1 | `GenieProvider` with Chat mode (GA, default), Genie One API (Beta), and Agent mode (Beta, "deep analysis" only) | M |
| G2 | Feature detection: `404 FEATURE_DISABLED` means fall back to Chat mode; probe Agent `execution_mode=FLASH` separately | M |
| G3 | Polling every 1 s with exponential backoff capped at 60 s, giving up at 10 minutes; own retries | M |
| G4 | SSE paths: save `response.id` and `conversation_id` from `response.created`; replace the item at `output_index` rather than appending; **never re-POST after a dropped connection** (recover with a GET); cancel endpoint wired to Stop | M |
| G5 | One in-flight question per conversation (handle 409); per-workspace token bucket; respect `Retry-After`; visible "Genie busy" state | M |
| G6 | New conversation per Mog session or meeting; optionally delete old ones to match Mog retention (cap 200,000 per agent) | S |
| G7 | Space picker using `GET /api/2.0/genie/spaces` (page_size ≤ 100) and `GET /api/2.0/genie/spaces/{id}` (title, description, sample questions), with favorites | M |
| G8 | Result card: answer text, table preview (row_count, is_truncated), viz PNG, SQL and thoughts, follow-ups, feedback, Open in Databricks, Copy as Markdown/CSV | M |
| G9 | Prerequisite check with clear explanations: CAN VIEW on the space, SELECT on its UC tables; for Genie One, an admin-enabled preview and the Consumer entitlement | M |
| G10 | Local question-to-answer cache and optional SQL templates for frequent KPIs through the Statement Execution API (GSK pattern: 64% less Genie load, 3–4% accuracy cost) | C |

### 3.7 Meeting mode
| ID | Requirement | Pri |
|---|---|---|
| MT1 | Started by the user only; detection only shows a prompt ("Zoom call detected, record?") | M |
| MT2 | Two tracks: mic ("You") and system loopback ("Others"); never pre-mixed | M |
| MT3 | macOS: Core Audio process tap + private aggregate device, `NSAudioCaptureUsageDescription`; signed binary required even in development; start the tap off the main thread with a timeout. ScreenCaptureKit fallback. | M |
| MT4 | Windows: WASAPI loopback (shared mode, `AUDCLNT_STREAMFLAGS_LOOPBACK`); process-loopback exclusion of Mog itself on build 20348+ | M |
| MT5 | Crash-safe storage: Ogg Opus 24–32 kbps per track, 5-minute rotating segments, pages flushed about every 1 s, JSON journal, recovery scan at launch | M |
| MT6 | Live transcription in VAD chunks of 30 s or less; chunk notes every 3–5 minutes; reduce pass at stop | M |
| MT7 | AEC on the mic track using system audio as the reference (OS voice processing live, or delay-aligned DTLN-aec offline) plus text-level dedup; nudge users toward headphones | S |
| MT8 | Signal-driven auto-stop policy (Section 5) | M |
| MT9 | Always-visible recording indicator (also when snoozed or in the tray); Pause/Resume/Stop one click away | M |
| MT10 | Consent: first-run legal notice, per-session "I have informed all participants" confirmation (admin-enforceable), "copy disclosure to chat" button, consent timestamp stored with the transcript | M |
| MT11 | v1 labels are You/Others only, with **no voiceprints** (BIPA). Diarization is opt-in, embeddings stay in memory for the session only and are never persisted. | M |
| MT12 | Meeting detection: macOS `kAudioHardwarePropertyProcessObjectList` + `kAudioProcessPropertyIsRunningInput` polled every 2 s (4 s start / 8 s end debounce, browser helpers mapped to the browser, Mog's own PID excluded); Windows ConsentStore `LastUsedTimeStop == 0` | S |
| MT13 | Optional read-only calendar (EventKit / Google / Graph) for the title, the end-time signal, and splitting back-to-back meetings | S |
| MT14 | "Send meeting to Genie" is a separate setting, **off by default**; only selected questions are sent, never the raw transcript or audio | M |

### 3.8 Security and governance
| ID | Requirement | Pri |
|---|---|---|
| S1 | OAuth U2M + PKCE (S256) via `https://<host>/oidc/v1/authorize` with a loopback redirect; per-account **public** custom OAuth app; scopes `all-apis offline_access` | M |
| S2 | No client secrets, service-principal credentials or PATs in the shipped binary (a PAT is a hidden developer fallback only); never use the `databricks-cli` client_id | M |
| S3 | Refresh token in Keychain / Credential Manager / libsecret via `keyring`; never logged; wiped on sign-out | M |
| S4 | Refresh TTL: keep the 7-day default, or have the admin raise it to ≤ 30 days **with** single-use refresh-token rotation (Beta); publish a revocation runbook | S |
| S5 | Every call uses the user's token, so UC row filters and masks apply and audit records show the real user (`aibiGenie` events) | M |
| S6 | Cleanup (and cloud ASR) routed through Unity Gateway model services; header `Databricks-Ai-Gateway-Request-Tags: {"app":"mog","mode":"dictation\|genie\|meeting"}`; service tag `app=mog` | M |
| S7 | Guardrails only work on non-streaming OpenAI Chat Completions and text: input PII guardrails on the fast path, output guardrails on Genie-mode and meeting-summary calls, plus a local deterministic PII check | S |
| S8 | Unity Gateway per-user daily budget for cleanup and ASR; Genie budget (`databricks-product: genie`); a clear "budget exhausted" state | S |
| S9 | Residency: tell admins about "Enforce data processing within workspace Geography"; Gemini audio needs cross-geo routing, so it is policy-gated; a policy can force local-only STT | M |
| S10 | Network reachability probe for IP access lists and PrivateLink, with "connect to VPN" messaging; honor `HTTPS_PROXY` and the system PAC | M |
| S11 | Telemetry is content-free by default (per-stage latency spans, model IDs, error codes) sent to OTLP `/api/2.0/otel/v1/traces` with `X-Databricks-UC-Table-Name`; content logging opt-in for the golden set; a separate opt-in crash reporter | S |
| S12 | Encryption at rest for local history and audio, with the key held in the OS keychain | M |
| S13 | When the Genie setting is on, the UI states that questions are stored in Genie history and audit logs | M |

### 3.9 Packaging and distribution
| ID | Requirement | Pri |
|---|---|---|
| P1 | macOS: Developer ID, hardened runtime, notarized DMG plus a PKG for MDM (no Mac App Store: event tap, `macOSPrivateApi`, audio taps) | M |
| P2 | Windows: Authenticode via **Azure Artifact Signing** (GA 2026-01-12, $9.99/month Basic; eligible orgs in US/CA/EU/UK); machine-wide MSI for MDM; expect SmartScreen warnings until reputation builds | M |
| P3 | Tauri updater with the signing key kept off build machines; staged channels with rollback | M |
| P4 | MDM policy keys: workspace URL, client_id, allowed Genie Agents, Genie setting allowed/forced off, meeting mode allowed, max duration, retention, local-only STT, audio deletion forced, consent forced | M |
| P5 | PPPC/DDM profile for Accessibility and PostEvent (Accessibility via PPPC is deprecated in 26.2 and removed in macOS 27); the microphone always needs one user click | S |
| P6 | Support floor: **macOS 14.4+** (14.2–14.3 best effort) and Windows 10 22H2+ (process-loopback exclusion only on 20348+, i.e. Windows 11; Windows 10 uses full-system loopback); features gated per OS | M |
| P7 | Test automation: WebdriverIO `@wdio/tauri-service` for UI E2E; a native harness and manual OS matrix for the event tap, panels, TCC, audio taps, DPI and sleep; 8-hour soak and 4-hour meeting test | M |
| P8 | MSIX or sparse-package identity for the Windows on-device SLM (Phi Silica now, Aion Instruct from January 2027) | C |

### 3.10 Mobile and web portability
| ID | Requirement | Pri |
|---|---|---|
| X1 | Domain logic (providers, state machine, Genie card components) is platform-neutral TS behind a transport interface | M |
| X2 | "Mog Pocket" for iOS: in-app recording (keyboard extensions cannot use the mic), a keyboard extension as trigger and inserter, Action button / Shortcuts / Control Center entry points, mic-only in-room meetings | C |
| X3 | Android: overlay bubble (`SYSTEM_ALERT_WINDOW`) plus an IME that records **[not deeply researched]** | C |
| X4 | Web: Genie and transcript UI through a Databricks App OBO backend (OBO is GA since 2026-10-07); no global hotkeys or cross-app paste; tab audio only via `getDisplayMedia` | C |

---

## 4. Decision tables

### 4.1 Desktop shell

| Criterion | **Tauri v2 + Rust core (PICK)** | Flutter desktop + native runner | Electron + native helpers |
|---|---|---|---|
| Push-to-talk key-up | Plugin reports Pressed/Released for chords; modifier-only and Fn need a native tap (Rust) | `hotkey_manager` 0.4: key-down only on Windows/X11; a custom `WH_KEYBOARD_LL` FFI hook is needed | `globalShortcut` has no key-up; needs `uiohook-napi` or native helpers (Wispr uses a Swift CGEventTap helper and a C# injector) |
| Non-activating panel | `tauri-nspanel` 2.1 (proven recipe) | No maintained plugin (`macos_floating_panel` is 0.0.1); subclass NSPanel in Swift | `type:'panel'`; the "activable" PR #52914 is still unmerged |
| Per-region click-through | Cursor poll + `set_ignore_cursor_events` | `setIgnoreMouseEvents(forward)`; one developer rejected Flutter for lacking selective click-through | `setInputShape`; Codex's Windows pet has a DPI click-through bug (#42061) |
| Multi-window | Stable | Official API experimental (main channel, `--enable-windowing`) | Stable |
| System audio | Rust Core Audio taps (Meetily) and WASAPI | `audio_io` WASAPI only; no macOS taps | `desktopCapturer` loopback with silent-failure bugs (#52738) |
| Reference app | **Handy** (MIT, Tauri 2, push-to-talk, Parakeet, VAD, overlay) | `dest_pets` proof of concept only | Codex/ChatGPT desktop, Wispr Flow |
| Size | ~4 MB on disk, ~64 MB idle | Medium | ~287 MB, multiple processes |
| Mobile/web | React UI reusable; Tauri mobile trails desktop | **Best** | None (separate app) |
| Risks | macOS transparency needs `macOSPrivateApi` (no MAS); DMG transparency bug #13415 **[spike]**; Rust skills | You own all native code; you are the first production dictation overlay on this stack | Heavy; focus-regression history |

**Pick: Tauri v2.** Put every OS-specific piece in Mog Core behind a C ABI. **Fallback:** Flutter, using the same core through `dart:ffi`, plus `window_manager`, `tray_manager` and `flutter_secure_storage`. Switch to it if mobile becomes the primary product.

### 4.2 ASR

| Option | Latency | Cost | Governance | Maturity | Verdict |
|---|---|---|---|---|---|
| **Local Parakeet TDT 0.6B v3 int8 (sherpa-onnx), FluidAudio on Mac** | 0.1–0.7 s CPU; 0.05–0.2 s ANE | $0 | No audio leaves the device; not logged in Databricks | Production-proven (Handy, VoiceInk-class apps); CC-BY-4.0 | **PICK (default for push-to-talk and meetings)** |
| **Gemini audio via Unity Gateway** (`databricks-gemini-3-1-flash-lite`, `audio_url` base64 data URI) | **[UNVERIFIED]** | Pay-per-token | Governed (usage, budgets, rate limits); requires cross-geo/global endpoint; 4 MB limit; no provisioned throughput | Documented | **Cloud pick, if the benchmark and verbatim-fidelity eval pass and cross-geo is approved** |
| Whisper large-v3-turbo, Custom LLM Serving (vLLM, GPU_MEDIUM A10G, always-on) | ~0.4–1.0 s warm (estimate) | ~20 DBU/h, about $1,022/month per replica | In-workspace; OpenAI `audio.transcriptions` | **Public Preview** (admin toggle, AI Runtime, MLflow 3.12+, databricks-sdk 0.102.0+); no dynamic autoscaling | Cloud fallback; in-region option |
| Parakeet/Nemotron on Databricks GPU (custom server) | Faster than Whisper | Same GPU cost | In-workspace | DIY packaging; vLLM support **[UNVERIFIED]** | v2 candidate if the shared endpoint scales |
| Vendor APIs direct (OpenAI `gpt-transcribe` $0.0045/min, `gpt-live-transcribe` $0.017/min, Deepgram, AssemblyAI) | Real streaming | Cheap per minute | **Bypasses Databricks governance**; external models do not support audio tasks | Mature; avoid whisper-1 and gpt-4o-*-transcribe (shut down 2027-02-26) | Optional, behind a policy toggle only |
| `ai_transcribe` (SQL, Beta) | Batch | AI_FUNCTIONS | Governed | Beta; English/Spanish only; 1 h / 512 MB per call; region-limited | Optional diarized meeting post-pass (needs a UC Volume upload tier) |

Cost check: for a team of under about 50 users, an always-on GPU endpoint costs 10–100x more than the whole cleanup bill. Local-first is cheaper as well as faster.

### 4.3 SLM (cleanup and Genie shaping)

| Model (endpoint) | Reasoning off? | Priority tier | Price (DBU per 1M tokens, in/out; $0.07/DBU) | Role |
|---|---|---|---|---|
| **`databricks-meta-llama-3-1-8b-instruct`** | Native non-reasoning | No | 2.143 / 6.429 (about $0.15 / $0.45) | **PICK: light cleanup, fast path** (about $0.45 per user per month at 200 utterances a day) |
| `databricks-gpt-6-luna` | `reasoning_effort=none` | Yes | **[price not researched]** | Contender for the fast path; **proposed Genie-mode pick** (Chat Completions, guardrail-compatible) |
| `databricks-gemini-3-1-flash-lite` | `reasoning_effort=minimal` | Yes (global endpoint) | **[not researched]** | Contender; cross-geo required |
| `databricks-claude-haiku-5-5` | `thinking.type=disabled` (default is medium) | No | ~$1 / $5 per 1M on Anthropic list pricing (Databricks rate unverified) | Heavy cleanup; Genie-mode fallback. Likely goes through the Anthropic Messages API, which guardrails do not evaluate. |
| `databricks-gemma-3-12b` | Non-reasoning | No | 2.143 / 7.143 | Fallback for multilingual users |
| `databricks-qwen3-next-80b-a3b-instruct` | Instruct | — | 2.143 / 17.143 | Public Preview; include in the eval only |
| `databricks-gpt-oss-20b` | Reasoning-only (14.7 s to first answer token at High) | — | 1.0 / 4.286 | **Not on the fast path.** Meeting chunk notes only. |
| Claude Sonnet 4.5/4.6 or Haiku 5.5 | — | — | — | Meeting reduce pass (about 12–16k tokens for 60 minutes) |
| Apple Foundation Models (macOS 26) | On-device | — | $0 | Offline tier (91.0% on one 300-case polish benchmark, about 0.9 s) |

**Pick:** Llama 3.1 8B for light cleanup, with Gemma 3 12B as fallback. Genie mode uses GPT-6 Luna (`none`, json_schema, non-streaming), with Haiku 5.5 (thinking off) as fallback. **Freeze these only after the golden-set eval.** If fast-path p95 exceeds 1 s on pay-per-token, move to provisioned throughput with a LoRA-tuned Llama 8B trained on captured edits, which is the Wispr approach. That needs user consent to store the edits.

### 4.4 Meeting default duration policy

| Option | Problem | Verdict |
|---|---|---|
| Flat 60 minutes (original proposal) | Cuts off long meetings and keeps recording hallway chatter after short ones; it also sits right at `ai_transcribe`'s 1 h cap | Rejected |
| Flat 90 minutes | Still truncates; ignores obvious end signals | Rejected |
| Signals only, no cap | Records forever if the signals fail | Rejected |
| **Signal-driven stop + 90-minute soft cap + 4-hour absolute ceiling** | — | **PICK** (Section 5) |

### 4.5 Genie backend and auth (summary)

| Choice | Pick | Fallback |
|---|---|---|
| Genie surface | Chat mode REST (GA) | Genie One API (Beta) once the probe passes; Agent mode only for "deep analysis" |
| Charts | GA download-visualization PNG | Native charts later; Genie One gives text and tables only (`enable_viz=false`) |
| Auth topology | Desktop to Databricks directly, U2M with PKCE | Databricks App OBO proxy if least-privilege scopes, central throttling or a web build are required |

---

## 5. Meeting mode default stop policy (replaces the flat 60 minutes)

**Principle:** stop when the meeting ends, not when a timer runs out. Never silently cut off speech that is still going on.

| # | Signal | Behavior |
|---|---|---|
| 1 | **Manual Stop or Pause** | Always available, one click, also from the tray. Pause does not end the session. |
| 2 | **Call app releases the mic** (Zoom, Teams, Meet in a browser, etc.) | After a 10 s debounce, stop. If no call app was detected (in-room meeting), this signal does not apply. |
| 3 | **Calendar end time** (if linked) | At end + 5 minutes, show a non-modal toast "Still in the meeting? Keep recording" with a 2-minute countdown. Recording continues automatically if VAD hears speech on either track, otherwise it stops. A back-to-back next event splits the session at the boundary. |
| 4 | **Silence on both tracks** | Nudge at 5 minutes ("Still recording?"); auto-stop at **15 minutes** (matches Granola). |
| 5 | **Sleep, lid close, logout, or disk below 500 MB** | Finalize the current segment and stop. A pre-flight disk check runs at start. |
| 6 | **Soft cap: 90 minutes** | Toast at 85 minutes: "Extend 30 min?" If the user does nothing **and speech is active**, extend by 30 minutes and show the toast again at each block. If there is no speech, stop at 90. |
| 7 | **Absolute ceiling: 4 hours** (admin-settable 1–8 h) | Hard stop, with check-in toasts at 180 and 225 minutes. Matches Otter Business and Fireflies Enterprise. |
| 8 | **Dictation modes** | Push-to-talk and hands-free dictation cap at 5 minutes (configurable). VAD end-of-utterance after 1.5–2 s of silence in hands-free mode. No audio retained. |

How the policy works with processing:
- Live chunks are 30 s or less. Any server-side batch job (`ai_transcribe`, a Whisper batch) must be **chunked under 1 hour** and FLAC/Opus-encoded.
- A 10-minute raw PCM window is about 19 MB, which is over the 16 MB limit.

Detection never starts recording on its own; it only offers a prompt.

---

## 6. UX spec: the mini, the mascot, and key bindings

### 6.1 Character and art pipeline
- **Commission an original "Mog" character.** The reference art (a blue pixel cloud robot with `>_`) resembles OpenAI's Codex mascot. Also run a trademark check on the name "Mog" and get approval for any Databricks marks.
- **Sprite contract: the Codex pet package format.**
  - Files: `pet.json` plus a transparent `spritesheet.webp`.
  - Grid: 8 columns of 192×208 cells. v1 is 9 rows (1536×1872); v2 adds look-direction rows (11 rows, 1536×2288).
  - Using this format means users can drop in existing pets and Codex tooling can validate the art.
- Render with nearest-neighbor filtering at integer scale (`image-rendering: pixelated` in the WebView).
- Optional upgrade: **Rive** (`rive` 0.14.11 on Flutter; Rive web runtime in Tauri) with data binding for mic RMS and Genie status. Transparent always-on-top rendering is **[UNVERIFIED; spike]**.

### 6.2 State machine (one machine drives the sprite, the pill and the tray icon)

| Mog state | Sprite row | Code-drawn overlay | Pill text | Trigger |
|---|---|---|---|---|
| idle / docked | idle (frame 0 = reduced-motion poster) | — | — | default |
| armed | waiting | dim ring | — | key down, mic not live yet |
| listening | waiting | **mic-RMS ring** + elapsed timer | "Listening" | first captured frame |
| transcribing / cleaning | running | spinner dots | "Transcribing…" / "Cleaning…" | key up |
| genie-thinking | review | status chip | `FETCHING_METADATA` → "Reading tables", `ASKING_AI` → "Thinking", `PENDING_WAREHOUSE` → "Waking warehouse", `EXECUTING_QUERY` → "Running SQL" | Genie request in flight |
| answer-ready | waving | speech bubble | "Answer ready" (+ OS notification) | COMPLETED |
| needs-input | jumping | "?" bubble | the clarifying question | FOLLOW_UP_QUESTION attachment |
| error | failed | — | short cause + fix action ("Reconnect", "Grant mic", "Connect to VPN", "Genie busy, queued") | any failure |
| meeting-recording | idle | **red badge + timer**, persistent | "Recording 00:42" | meeting start |
| offline / queued | idle (desaturated) | cloud-off glyph | "Will ask when online" | reachability probe fails |

Rules:
- The sprite is never the only indicator. Earcons, the pill text, the tray icon and accessibility announcements carry the same state.
- A recording dot stays visible even when the mini is snoozed.
- Mog's indicator must never contradict the OS mic indicators.
- The mini can be dragged, its position is saved, and it re-asserts topmost after display changes.
- Hovering shows Stop / Expand. Right-click opens History, Settings and the workspace and space switcher.
- The card panel is the only surface that takes focus.

### 6.3 Key bindings (all rebindable; conflict-checked at first run)

| Action | macOS default | Windows default | Notes |
|---|---|---|---|
| Push-to-talk (hold) / hands-free (double-tap within 0.5 s) | **Right-Option** | **Right-Alt** on US layouts; **Ctrl+Win+M** on AltGr layouts | Avoids Wispr's defaults (Fn, Ctrl+Win) and Option+Space (ChatGPT/Codex Mini, Superwhisper). Fn offered as an option after an `AppleFnUsageType` check and a "Set Globe to Do Nothing" guide. On many European layouts Right-Alt is AltGr (general knowledge; check per layout). |
| Ask Genie (per utterance) | **Shift + Right-Option** (hold) | **Shift + Right-Alt** / Ctrl+Win+Shift+M | Holding Shift at key-down routes to Genie: a deliberate consent moment |
| Cancel | Esc | Esc | Any mode; a press under 0.5 s cancels with a "hold, don't tap" hint; a third quick press cancels silently |
| Paste last | Ctrl+Option+V | Ctrl+Win+Alt+V | |
| Copy last | Ctrl+Option+C | Ctrl+Win+Alt+C | |
| Meeting start/stop | Ctrl+Option+M | Ctrl+Win+Alt+R | |
| Open card / history | Ctrl+Option+Space | Ctrl+Win+Alt+Space | |

Further rules:
- At most 3 keys, at least one modifier for chords, reserved combinations rejected (Cmd/Ctrl+C/V/X/Z, Cmd+Tab).
- Pressing another key within the first second of a hold cancels it.
- Toggle mode is available as an accessibility alternative to holding.
- All the defaults above are proposals and are **[UNVERIFIED against every OS and app conflict]**.

### 6.4 The Genie setting (resolving the auto-send conflict)
- **Off (default):** Mog only dictates. Shift+push-to-talk still asks Genie explicitly.
- **On:** a data question (`intent=data_question`, `confidence ≥ 0.7`) gets a 1.5 s preview with a cancel option and is then sent. Anything else gets an "Ask Genie" chip. The mini shows the target Genie Agent name.
- **Meeting to Genie:** a separate setting, off by default. Questions are shown unchecked and sent one at a time.
- Admins can force it off or require the preview.

---

## 7. Risks, mitigations and open questions

### 7.1 Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Genie throughput is unpublished (historically about 5 questions/min via the API and 20/min in the UI per workspace; those figures come from stale Microsoft Learn pages) | 429s across the team, especially from meeting mode | Client token bucket, queue, `Retry-After`, no auto-send, user-selected meeting questions, optional cache; confirm limits with the account team |
| Genie latency of 10–60 s, plus about 4 minutes for a non-serverless warehouse | Feels broken against an "instant" promise | Treat Genie as async (status chips, notification); warn during onboarding if the warehouse is not serverless |
| Agent mode and the Genie One API are Beta; schemas may change; unknown fields are rejected | Breakage | Feature detection, pinned request shapes, Chat-mode fallback |
| Custom LLM Serving (Whisper) is Public Preview and GPUs may not be available in the region | Cloud ASR unavailable | Local-first; Gemini audio tier; admin onboarding checklist |
| Gemini audio may "answer" or paraphrase instead of transcribing; needs cross-geo routing | Fidelity or residency violation | Verbatim eval against Parakeet/Whisper; strict transcription prompt; gated by policy |
| Cleanup model answers the question or changes numbers and filters | Wrong dictation or a corrupted Genie question | Few-shot guardrails, entity/number preservation check, raw fallback, raw toggle, eval gate on the violation rate |
| Model retirements (DeepSeek V4.1 Flash 2026-11-19; Gemini 3.6 Flash redirected to 3.8, which rejects `minimal`; GLM 5.2 2027-02-04) | Hard failures or behavior changes | Model names as config, fallback chains, a startup probe |
| Streaming vs. guardrails: output guardrails only run on non-streaming Chat Completions | No PII masking on the fast path | Input guardrails plus a local PII check on the fast path; output guardrails on Genie and summary calls |
| macOS TCC friction; no public API to check or request System Audio Recording; unsigned builds silently get zeros | Silent meeting recordings | Signed developer builds, zero-buffer detection plus a fix-it UI, onboarding wizard; avoid private TCC APIs (notarization risk) |
| Fn/Globe collides with emoji, input-source or dictation actions; firmware Fn on external keyboards | Hotkey unreliable | Right-Option default; active tap; `AppleFnUsageType` detection |
| Topmost or click-through overlay bugs (Codex #22708, #42061) | Lost mascot, blocked clicks | Topmost watchdog; re-apply hit regions on display or DPI events; tray fallback |
| Tauri DMG transparency bug #13415 | Opaque mascot | Week-1 spike with a signed, notarized build in CI |
| Paste fails in secure fields, elevated (UIPI) windows, VS Code or terminals | Text lost | AX/UIA verify, clipboard fallback, copy card, confirmation in the pill |
| Clipboard history and cloud sync capture transcripts | DLP leak | Transient and concealed clipboard flags |
| Meeting recording law (all-party consent states vary between sources; GDPR); Otter BIPA/CIPA claims survived dismissal in 2026 | Legal exposure | Treat every meeting as all-party consent; attestation; disclosure snippet; no voiceprints; audio deleted by default; enterprise controls |
| Echo on laptop speakers duplicates remote speech in the "You" track | Bad transcript and Genie context | AEC with the system reference, text dedup, headphone nudge |
| Refresh tokens on laptops (all-apis scope, 7-day default TTL) | Broad blast radius if a laptop is stolen | Keychain only, single-use rotation, moderate TTL, revocation runbook, OBO proxy if least privilege is mandatory |
| IP access lists / PrivateLink | Every call fails off VPN | Reachability probe and VPN messaging; proxy/PAC support |
| Cost shift after 2027-01-31 (Genie: 150 DBU per user per month free, then $0.07/DBU plus warehouse) | Budget surprises | Genie budget tag, per-user caps, short conversations, warehouse auto-stop |
| Parakeet RAM (0.75–0.95 GB) in an always-running app | Battery and RAM complaints | Unload after 15 minutes idle; small-model tier (Moonshine v2 Small/Medium, MIT) |
| Two native STT engines on Mac (FluidAudio and sherpa-onnx); ANE fragile on OS betas | Larger test surface | sherpa-onnx on every OS in the MVP; FluidAudio added as an optional fast path in v1 |
| Mascot IP similarity to Codex | Trademark or brand dispute | Original character; legal review |

### 7.2 Open questions and unverified items
1. Current numeric throughput limits for Genie Chat mode, Agent mode and the Genie One API, and whether they change after 2027-01-31. *(Account team.)*
2. Whether a custom OAuth app can register fine-grained scopes (`genie`, `model-serving`, `ai-gateway`) in practice. The docs say no; one third-party reference suggests yes.
3. Whether Genie usage from an external desktop client with a U2M token counts toward the free user allowance in the same way as UI usage. Assumed, not verified.
4. Latency and verbatim fidelity of Gemini audio input on Databricks for 5–10 s clips, and whether the org approves cross-geo routing.
5. Measured p50/p95 TTFT from the desktop for Llama 3.1 8B, GPT-6 Luna, Gemini 3.1 Flash-Lite and Haiku 5.5 (thinking off).
6. Whether Haiku 5.5 with thinking disabled is reachable through OpenAI Chat Completions, which decides guardrail eligibility.
7. Parakeet v3 redistribution license: CC-BY-4.0 on Hugging Face versus the NVIDIA Open Model License cited on build.nvidia.com. *(Legal.)*
8. sherpa-onnx hotword biasing with Parakeet TDT and Nemotron; Parakeet int8 speed on CPU-only Windows/Linux and Windows on ARM laptops.
9. Whether onnxruntime's CoreML execution provider in sherpa-onnx v1.13+ needs macOS 14.4+ (one third-party report).
10. Whether Tauri #13415 is fixed in 2.12.x; whether Rive renders on a transparent, per-pixel-alpha always-on-top window.
11. The query parameter for `serialized_space`, and whether CAN VIEW users can read it (needed for the glossary).
12. Whether Databricks APIs allow browser CORS (affects the web build; assume a proxy is needed).
13. Whether Model Serving or Databricks Apps accept WebSocket ingress for streaming audio. Assume not.
14. Whether Opus at 24–32 kbps is transparent to ASR versus FLAC on internal meeting audio.
15. Product and legal decision: may meeting transcripts or summaries go to Genie at all, and may captured edits be used for fine-tuning?

---

## 8. Phased roadmap

### MVP: dictation plus asking Genie (macOS 14.4+ first, Windows 11 close behind)
- Tauri v2 shell; Mog Core in Rust (active CGEventTap / `WH_KEYBOARD_LL`, cpal, Silero v6.2.x, sherpa-onnx Parakeet v3 int8, AX then clipboard insertion, keyring).
- Week-1 spikes: signed DMG transparency (#13415), `tauri-nspanel` non-activating mascot, event-tap watchdog, Parakeet latency on target laptops.
- OAuth U2M + PKCE with the customer's custom public app; workspace and space picker.
- Cleanup: pre-pass plus `databricks-meta-llama-3-1-8b-instruct` through a Unity Gateway model service; light and verbatim levels; raw toggle; Paste-last.
- Genie: Chat mode only; status chips; text, table and viz PNG card; Shift+push-to-talk and the setting with preview.
- Placeholder original mascot in the Codex atlas format; 8 core states; reduced motion.
- Golden set (≥ 150 utterances) and an MLflow eval harness; content-free OTLP latency spans.
- Admin onboarding checklist: OAuth app, CAN VIEW and SELECT grants, serverless warehouse, Unity Gateway service.

### v1: meetings and enterprise readiness
- Meeting mode: Core Audio taps and WASAPI loopback, two tracks, Ogg Opus journal and recovery, live local chunks, chunk notes plus a reduce pass, the Section 5 stop policy, consent flow, meeting detection, optional calendar, You/Others labels.
- Genie One API path (after the probe), Agent mode "deep analysis", queueing and 429 handling.
- Cloud ASR tier: Gemini audio (if the eval passes) or the always-on Whisper-turbo endpoint.
- Heavy and Genie cleanup modes (GPT-6 Luna / Haiku 5.5 per the eval); glossary from the space and UC; output guardrails on non-streaming calls.
- FluidAudio ANE fast path on Mac; Apple Foundation Models offline cleanup on macOS 26.
- MDM policy keys, notarized PKG, MSI signed with Azure Artifact Signing, Tauri updater channels, PPPC/DDM docs.
- Reachability and proxy handling, multi-workspace profiles, hotkey conflict matrix, accessibility announcements, i18n scaffolding (i18next).
- AEC, budgets and request tags, history retention tiers.

### v2: scale, quality and reach
- Provisioned-throughput LoRA-tuned Llama 8B cleanup model trained on consented edit triples (if p95 is over 1 s).
- Opt-in diarization with session-only embeddings; optional `ai_transcribe` post-pass through an admin-gated UC Volume tier; opt-in sync of transcripts to UC.
- Shared Parakeet/Nemotron GPU endpoint if the cloud ASR load justifies it; optional live captions (Nemotron Speech Streaming 0.6B, 560 ms chunks).
- Rive mascot with data binding; user-supplied pets; native charts.
- Genie Q&A cache and KPI SQL templates; Databricks App OBO proxy for org-scale throttling, least-privilege scopes and the web build.
- "Mog Pocket" (iOS in-app recording plus a keyboard extension; Android bubble and IME) reusing the TS domain layer; Windows on-device SLM (Aion Instruct) via MSIX identity; Linux X11 best-effort.

---

## 9. Sources

**Genie and Databricks platform**
- https://docs.databricks.com/aws/en/genie/conversation-api
- https://docs.databricks.com/aws/en/genie-agents/conversation-api
- https://docs.databricks.com/api/genie/v1/genie-start-conversation.md
- https://docs.databricks.com/api/genie/v1/genie-get-message-attachment-query-result.md
- https://docs.databricks.com/api/workspace/genie
- https://docs.databricks.com/api/genie/v1/agent-mode-create-response
- https://docs.databricks.com/aws/en/genie-one/api
- https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp
- https://docs.databricks.com/aws/en/generative-ai/mcp/managed-mcp
- https://docs.databricks.com/aws/en/resources/limits
- https://docs.databricks.com/aws/en/genie/best-practices
- https://docs.databricks.com/aws/en/genie/monitor-cost
- https://docs.databricks.com/aws/en/genie/budgets
- https://www.databricks.com/product/pricing/genie
- https://docs.databricks.com/aws/en/dev-tools/cli/reference/genie-commands
- https://learn.microsoft.com/zh-cn/azure/databricks/genie/set-up (historical throughput figures)
- https://learn.microsoft.com/fr-fr/azure/databricks/genie/set-up
- https://community.databricks.com/t5/generative-ai/genie-spaces/td-p/160123
- https://answers.databricks.com/scaling-genie-production-gsk-s-enterprise-ai-bi-factory-bhA6leq4BdY
- https://www.databricks.com/blog/access-genie-everywhere
- https://rohitbhagwat.medium.com/calling-genie-agent-mode-with-the-openai-client-fb134649911b
- https://docs.dbx.tools/packages/genie/
- https://github.com/databricks/sdk-js/
- https://databricks.github.io/sdk-js/api/classes/_databricks_sdk-genie.v1.GenieClient.html
- https://github.com/databricks/databricks-sdk-py/blob/main/docs/workspace/dashboards/genie.rst

**Auth and governance**
- https://docs.databricks.com/aws/en/dev-tools/auth/oauth-u2m
- https://docs.databricks.com/aws/en/integrations/enable-disable-oauth
- https://docs.databricks.com/api/o-auth/v1/create-custom-o-auth-app-integration.md
- https://docs.databricks.com/api/o-auth/v1/custom-app-integration
- https://www.databricks.com/blog/now-ga-building-permission-aware-databricks-apps-behalf-user-authorization
- https://databrickslabs.github.io/partner-architecture/isv-partners/integration-requirements
- https://docs.databricks.com/aws/en/admin/account-settings/audit-logs
- https://www.databricks.com/blog/ai-gateway-governance-layer-agentic-ai
- https://docs.databricks.com/aws/en/ai-gateway/overview-serving-endpoints
- https://docs.databricks.com/aws/en/ai-gateway/configure-ai-gateway-endpoints
- https://docs.databricks.com/aws/en/ai-gateway/model-services
- https://docs.databricks.com/aws/en/ai-gateway/track-cost-tutorial
- https://docs.databricks.com/aws/en/ai-gateway/budgets
- https://docs.databricks.com/aws/en/admin/usage/budget-policies
- https://docs.databricks.com/aws/en/resources/designated-services
- https://docs.databricks.com/aws/en/resources/databricks-geos
- https://github.com/juliansteenbakker/flutter_secure_storage/

**Model Serving, ASR and SLM**
- https://docs.databricks.com/aws/en/machine-learning/model-serving/serve-custom-llms
- https://docs.databricks.com/aws/en/machine-learning/model-serving/query-audio-video-models
- https://docs.databricks.com/aws/en/machine-learning/model-serving/model-serving-limits
- https://docs.databricks.com/aws/en/machine-learning/model-serving/model-serving-timeouts
- https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/limits
- https://docs.databricks.com/aws/en/machine-learning/foundation-models/external-models/
- https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/supported-models
- https://docs.databricks.com/aws/en/machine-learning/model-serving/foundation-model-overview
- https://docs.databricks.com/aws/en/machine-learning/model-serving/query-reason-models
- https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/priority-mode
- https://docs.databricks.com/aws/en/machine-learning/model-serving/structured-outputs
- https://www.databricks.com/product/pricing/model-serving
- https://www.databricks.com/product/pricing/foundation-model-serving
- https://www.dbuwatch.com/guides/databricks-model-serving-pricing/
- https://kb.databricks.com/machine-learning/ssl-error-when-invoking-databricks-model-serving-endpoint
- https://docs.databricks.com/aws/en/sql/language-manual/functions/ai_transcribe
- https://github.com/BerriAI/litellm/pull/43168
- https://developers.openai.com/api/docs/pricing
- https://developers.openai.com/api/docs/guides/transcription
- https://developers.openai.com/api/docs/guides/realtime-transcription
- https://openvoiceindex.com/api-prices/
- https://artificialanalysis.ai/models/comparisons/claude-4-5-haiku-vs-gpt-oss-20b
- https://www.baseten.co/resources/customers/wispr-flow/
- https://wisprflow.ai/post/technical-challenges
- https://www.cubic.dev/wikis/Beingpax/VoiceInk?page=ai-enhancements
- https://tryvoiceink.com/docs/filler-words
- https://github.com/metaforismo/scriba/blob/main/ROADMAP.md
- https://www.frontiersin.org/journals/artificial-intelligence/articles/10.3389/frai.2025.1604034/full
- https://enviouswispr.com/blog/on-device-dictation-polishing-small-models/

**Local STT, VAD and diarization**
- https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
- https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2
- https://amical.ai/blog/parakeet-tdt-v3
- https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b
- https://huggingface.co/nvidia/parakeet-unified-en-0.6b
- https://github.com/k2-fsa/sherpa-onnx
- https://github.com/k2-fsa/sherpa-onnx/releases/tag/v1.13.0
- https://github.com/k2-fsa/sherpa-onnx/blob/master/flutter-examples/README.md
- https://pub.dev/packages/sherpa_onnx/versions/2.0.0/example
- https://pub.dev/packages/sherpa_onnx_windows/example
- https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/models.html
- https://github.com/FluidInference/FluidAudio/
- https://docs.fluidinference.com/introduction
- https://github.com/quent1nQ/muesli
- https://github.com/anvanvan/mac-whisper-speedtest
- https://talkhawk.app/dictation-on-a-cpu
- https://www.owntools.app/blog/parakeet-vs-whisper-cpu
- https://pinggy.io/blog/best_open_source_self_hosted_speech_to_text_models/
- https://opencodepapers-b7572d.gitlab.io/benchmarks/speech-recognition-on-open-asr-leaderboard.html
- https://arxiv.org/html/2602.12241
- https://moonshine-voice.readthedocs.io/en/stable/license/
- http://github.com/ggml-org/whisper.cpp
- https://bisque.today/p/whisper-cpp/whisper-cpp-1-9-5
- https://pub.dev/packages/whisper_ggml
- https://github.com/senstella/parakeet-mlx
- https://github.com/kyutai-labs/delayed-streams-modeling/
- https://github.com/snakers4/silero-vad/releases
- https://github.com/snakers4/silero-vad/wiki/Performance-Metrics
- https://github.com/pyannote/pyannote-audio
- https://github.com/cjpais/transcribe-rs
- https://github.com/cjpais/Handy/discussions/836
- https://prometheus-maker.github.io/reader-board/vosk-vs-whisper.html

**Desktop shell, hotkeys, overlays and insertion**
- https://github.com/cjpais/Handy/
- https://github.com/cjpais/Handy/blob/main/AGENTS.md
- https://v2.tauri.app/plugin/global-shortcut/
- https://v2.tauri.app/develop/tests/webdriver/
- https://github.com/ahkohd/tauri-nspanel/
- https://unwait.ai/blog/macos-overlay-that-never-steals-focus
- https://txz.cool/blog/m36p2ntvjgaazq07pg08wix5
- https://blog.manasight.gg/why-i-chose-tauri-v2-for-a-desktop-overlay/
- https://vipertecpro.com/blog/tauri-vs-electron-2026
- https://www.urhoba.net/en/post/tauri-mobile-android-ios
- https://pub.dev/packages/hotkey_manager
- https://pub.dev/packages/window_manager
- https://pub.dev/packages/macos_floating_panel
- https://flutter.dev/blog/desktop-windowing-apis
- https://flutter.dev/blog/whats-new-in-flutter-3-44
- https://github.com/lizy-coding/dest_pets
- https://github.com/electron/electron/issues/7802
- https://github.com/electron/electron/pull/52914
- https://codenote.net/en/posts/famous-electron-apps-2026-research/
- https://seanthegeek.net/doubleagent/research/chatgpt-desktop.html
- https://learn.chatgpt.com/docs/pets
- https://www.digitaltrends.com/computing/openais-codex-now-has-a-tiny-ai-pet-that-keeps-you-updated-while-you-code/
- https://github.com/openai/codex/issues/42061
- https://github.com/openai/codex/issues/22708
- https://github.com/elizaos/eliza/pull/20613
- https://github.com/wispr-flow-linux/wispr-flow-linux/blob/ea577da7/AGENTS.md
- https://www.raycast.com/blog/a-technical-deep-dive-into-the-new-raycast
- https://jbmartino.com/posts/rebuilding-wisprflow-locally/
- https://fnflow.app/fn-key-dictation-mac
- https://docs.mumbli.app/for-developers/accessibility-permissions
- https://docs.wisprflow.ai/articles/2012729330-why-your-dictation-was-cancelled-tap-vs-hold-and-interrupted-starts
- https://docs.wisprflow.ai/articles/2612050838-supported-unsupported-keyboard-hotkey-shortcuts
- https://superwhisper.com/docs/get-started/essential-settings
- https://superwhisper.com/docs/get-started/interface-rec-window

**Audio capture and meeting mode**
- https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps
- https://victorantos.com/posts/macos-process-taps-tapelark/
- https://dgrlabs.co/blog/2026-04-25-capturing-system-audio-on-macos-in-2026.html
- https://stronglytyped.uk/articles/recording-system-audio-electron-macos-approaches
- https://zoomcap.live/blog/screencapturekit-audio
- https://deepwiki.com/Zackriya-Solutions/meetily/3.2-audio-capture-backends
- https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/
- https://learn.microsoft.com/en-us/windows/win32/coreaudio/audclnt-streamflags-xxx-constants
- https://releases.electronjs.org/pr/53646
- https://github.com/electron/electron/issues/52738
- https://www.reline.so/blog/how-to-record-system-audio-on-linux
- https://macnotetaker.com/blog/which-app-is-using-mic-coreaudio-process-objects
- https://systemaudiokit.lucaspiera.com/docs/meetings
- https://www.windowsforensics.app/en/artifacts/capability-access-manager
- https://github.com/r3dbars/transcripted/pull/1074
- https://github.com/vasovagal/corti/issues/64
- https://peritissimus.com/blog/local-meeting-transcription-whisper-cpp
- https://pub.dev/packages/record
- https://pub.dev/packages/audio_io
- https://pub.dev/documentation/audio_io/latest/
- https://docs.granola.ai/help-center/taking-notes/transcription
- https://www.granola.ai/blog/why-granola-doesnt-use-a-bot
- https://www.granola.ai/blog/ai-notetaker-participant-privacy-consent
- https://www.notion.com/help/ai-meeting-notes
- https://otter.ai/pricing
- https://guide.fireflies.ai/articles/9592347753-how-long-can-fireflies-stay-in-your-meetings

**Legal, enterprise deployment and signing**
- https://www.findlaw.com/employment/workplace-privacy/is-it-legal-to-record-your-boss-or-coworkers.html
- https://www.recordinglaw.com/tools/recording-consent-checker/
- https://www.sheppard.com/insights/blogs/when-ai-takes-notes-court-allows-privacy-claims-against-otterai-to-proceed
- https://natlawreview.com/article/take-note-new-wave-privacy-litigation-targets-ai-notetaker-otterai
- https://wisprflow.ai/enterprise-privacy
- https://docs.wisprflow.ai/articles/9363440133-deploy-wispr-flow-via-mdm
- https://www.manageengine.com/mobile-device-management/help/profile_management/mac/mdm_mac_pppc.html
- https://www.ssmacadmin.com/posts/2026-09-30-macos27-intune-privacy-and-binary-control/
- https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options
- https://support.microsoft.com/en-us/windows/privacy/windows-camera-microphone-and-privacy