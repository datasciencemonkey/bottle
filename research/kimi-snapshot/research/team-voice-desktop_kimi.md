# Motherbricks Team V: voice and desktop requirements

Date: 2026-11-14  
Owner: Team V, voice and desktop  
Status: requirements and verification plan for director integration  
Scope: D-01, D-02, D-05, D-09, D-10, D-11 and D-12  
Naming: **Motherbricks** is the project and desktop app. A **MOG** is a floating agent created by Motherbricks.

## 1. Evidence boundary

This report uses primary-source research accessed on 2026-11-14.
The current host runs Linux.
No macOS or Windows device was available.
No physical-device spike was run.
No benchmark was run.
No timing, accuracy, energy, permission, focus, or audio-capture result is claimed.
Every such result below is **PENDING**.
The pending items are release gates for G1, G2 and the Team V exit tests.

This report uses ASD-STE100-style simplified English.
It does not claim formal ASD-STE100 compliance.
The word **shall** marks mandatory behavior.
The word **should** marks recommended behavior.
The word **may** marks permitted behavior.

## 2. Decisions and assumptions

| ID | Decision or assumption | Effect |
|---|---|---|
| DV-01 | Use Tauri v2, React, TypeScript and a native Rust service for the desktop MVP. | D-01 remains unchanged. |
| DV-02 | Test the current Tauri `global-shortcut` backend before adding a native keyboard hook. | Current primary documentation includes `Pressed` and `Released`. This removes the earlier reason to assume that the plugin cannot support push-to-talk. |
| DV-03 | Use a native hook only when the spike proves a need. | Modifier-only bindings, event suppression, or failed release delivery may need `CGEventTap` or `WH_KEYBOARD_LL`. |
| DV-04 | Use a non-activating mascot surface and a separate keyboard-capable response panel. | The mascot does not steal the user's text destination. |
| DV-05 | “Always available” does not mean continuous microphone capture. | No transcript, memory, calendar event, backend response, or remote command can start capture. |
| DV-06 | Local Parakeet TDT 0.6B v3 int8 with sherpa-onnx is the default STT candidate. | It supports offline dictation after model installation. The sherpa-onnx model is non-streaming, so the UI promises chunk results, not token-streaming ASR. |
| DV-07 | Use the sherpa-onnx converted model and pin all model artifacts by digest. | NVIDIA model-card results do not prove ONNX Runtime latency or accuracy on Motherbricks hardware. |
| DV-08 | Databricks-hosted STT is optional and configurable. | Custom Model Serving documents Whisper transcription in Public Preview. Gemini audio is a second candidate. Neither is a hidden default. |
| DV-09 | Use `system.ai.meta-llama-3-1-8b-instruct` or endpoint `databricks-meta-llama-3-1-8b-instruct` as the first cleanup evaluation candidate. | Workspace availability and permissions still need a runtime probe. No hidden model substitution is allowed. |
| DV-10 | Keep raw, cleaned, and user-corrected text as separate values. | Raw text is never destroyed by cleanup or user correction. |
| DV-11 | Insert one selected text version once. | A late cleanup result can update the Motherbricks review card, but it cannot edit another application. |
| DV-12 | Motherbricks owns every capture lease. | A MOG never claims an audio device or global binding on its own. |
| DV-13 | Use one exclusive microphone lease across push-to-talk, hands-free, and meeting modes. | This is stricter than D-11 and prevents a meeting plus a dictation session from capturing the same microphone for different MOGs. |
| DV-14 | Capture process audio only from the user-selected call process tree. | Do not silently fall back to whole-system audio. |
| DV-15 | Use source labels `microphone` and `system`, not speaker identities. | A microphone can capture more than one person. A system track can contain more than one person. |
| DV-16 | Treat call-application microphone state as `Active`, `Released`, or `Unknown`. | No universal cross-application signal was verified. `Unknown` is not `Released`. |
| DV-17 | Keep the director's 15-minute silence, 90-minute review, and four-hour ceiling. | No evidence supports changing these values before the physical-device study. |
| DV-18 | Add a 120-second push-to-talk safety limit. | This bounds capture when an operating system or keyboard fails to deliver key-up. |
| DV-19 | Use crash-safe Ogg Opus journal segments. | ASR chunks are separate from storage segments. This avoids one large fragile recording file. |
| DV-20 | Desktop MVP target is macOS 14.4 or later on Apple Silicon and Windows 11 23H2 or later on x64. | This is a proposed support floor. It is not a claim of tested support. Linux, mobile, and web are later targets. |
| DV-21 | Enable English only for the first local-STT release evaluation. | The model supports 25 languages, but each product language needs its own corpus gate. |
| DV-22 | Use the 1500 ms cleanup provider wait as a sub-budget within Architecture's 2.0-second overall cleanup deadline. | Local validation and raw fallback must finish inside the outer deadline. The two team budgets are compatible. |

## 3. Load-bearing evidence and remaining uncertainty

| Claim | Status | Primary evidence | Requirement effect |
|---|---|---|---|
| Tauri global shortcut events include pressed and released states. | Verified in current documentation | S-01 | Test the plugin first. Physical release behavior remains pending. |
| `isRegistered` does not detect another application's registration. | Verified in current documentation | S-01 | Motherbricks must reject its own duplicate bindings. It must also handle operating-system registration failure. |
| macOS has key-down, key-up, modifier-change, and event-tap-disabled event types. | Verified | S-02 | A native fallback must handle modifier release and disabled taps. |
| Windows low-level hooks include key-up and can be removed silently after timeout. | Verified | S-03 | The callback must return quickly. Push-to-talk still needs a safety limit. |
| macOS process taps capture output from selected processes and use audio-capture consent. | Verified in Apple sample documentation | S-04 | Process-scoped capture is feasible to spike. The sample states macOS 14.2 or later. Motherbricks proposes a 14.4 support floor. |
| Windows process loopback can include a process tree. | Verified | S-05 | Use process-scoped capture. Build 20348 is the API floor, not the proposed product support floor. |
| Windows input injection is blocked across some integrity levels. | Verified | S-06 | Automatic insertion can fail. Copy fallback is mandatory. |
| Windows clipboard changes have a sequence number. | Verified | S-07 | Do not restore old clipboard content after a newer change. |
| NVIDIA Parakeet TDT 0.6B v3 supports 25 European languages, punctuation, capitalization, and 16 kHz mono input. | Verified for the NeMo model | S-08 | It is a suitable candidate. Converted-runtime capability still needs a spike. |
| NVIDIA's long-audio figures use full attention on A100 or local attention. | Verified | S-08 | Do not use those figures to promise long local ONNX inference. Use bounded ASR chunks. |
| sherpa-onnx publishes a Parakeet TDT 0.6B v3 int8 conversion and microphone-plus-VAD examples. | Verified | S-09 | This is the default local runtime candidate. Its published artifact list is about 640 MiB before runtime overhead. |
| Current sherpa-onnx VAD documentation lists Silero v4 and v5. | Verified | S-10 | Do not require the earlier unverified “Silero v6.2.x” name. Pin a supported VAD artifact after the evaluation. |
| Databricks Custom Model Serving supports Whisper large-v3-turbo and `llm/v1/audio/transcriptions`. | Verified as Public Preview | S-11 | It is optional. It needs an admin-enabled preview and a warm endpoint for interactive use. |
| Databricks Gemini pay-per-token models accept inline or URL audio. | Verified | S-12 | Use inline audio for the evaluation. Verbatim fidelity, timestamps, diarization, and latency remain unverified. |
| Databricks Chat Completions supports temperature, bounded output, and structured output on supported models. | Verified | S-13 | Use bounded constrained cleanup with raw fallback. Structured output alone does not prove semantic fidelity. |
| Databricks lists `system.ai.meta-llama-3-1-8b-instruct` and `databricks-meta-llama-3-1-8b-instruct`. | Verified in the model catalog | S-15 | Use this as the first cleanup candidate. Workspace availability and latency remain pending. |
| `ai_transcribe` supports timestamped segments and diarization in Beta. | Verified, but not a desktop serving API | S-14 | Do not put this SQL/notebook capability on the interactive desktop path. |
| Cross-application microphone release has one supported signal on every target OS and call app. | Unverified | No primary source found for a universal signal | Use a capability provider. Never infer `Released` from silence or process activity. |
| Parakeet through sherpa-onnx supplies calibrated word confidence on every MVP platform. | Unverified | S-08 and S-09 do not establish this | Report confidence capability honestly. Do not invent scores. |
| Tauri plus native window code can meet focus, overlay, permission, and insertion requirements on reference devices. | Unverified | S-01 through S-07 establish APIs, not integrated behavior | V-SPK-01, V-SPK-02 and V-SPK-05 block release. |

## 4. Capture behavior

### 4.1 Modes

| Mode | Start | Normal end | Cancel or interruption | Data result |
|---|---|---|---|---|
| Push-to-talk | The user presses the complete configured binding. | The binding is no longer fully held. | Escape, foreground-target change, permission loss, source loss, or 120-second limit. | Normal end transcribes the utterance. Cancel does not insert or submit it. |
| Hands-free dictation | The user selects Start dictation or a separate configured binding. | The user selects Stop or the same explicit binding. | Escape cancels the current utterance. Permission or source loss interrupts the session. | Stop transcribes the current utterance. A visible review card holds completed text. |
| Meeting | The user selects Start meeting after source and target review. | The user selects Stop. | A required stop signal ends capture. Device recovery and sleep follow Sections 5 and 8. | Completed journal segments remain available for transcription and recovery. |

A double-tap binding is not an MVP start mechanism.
It can be evaluated later as an optional accessibility and usability feature.

### 4.2 Meeting signals

| Signal | Required behavior |
|---|---|
| Manual Stop | Stop new capture. Close the current journal segment. Preserve completed data. |
| 60 minutes elapsed | Show elapsed time. Continue capture. |
| 15 minutes without VAD speech on every active track | Show a 60-second countdown. Stop when the countdown reaches zero. Reset the countdown when speech resumes. |
| 90 minutes elapsed | Show a review notice. Continue when meeting signals remain active. |
| Calendar end | Ask the user whether to continue. Do not stop for this signal alone. |
| Trusted call-signal provider reports microphone release | Start a 30-second grace period. Stop if the reported state remains released. Cancel the grace period if it reports active again. |
| Call-signal provider reports `Unknown` | Do not start the release grace period. Continue to use manual, silence, sleep, disk, and hard-ceiling policies. |
| System sleep | Stop capture. Close what can be closed safely. Mark the session `interrupted_by_sleep`. Do not resume automatically. |
| Free disk space reaches 2 GiB | Show a low-disk warning. Continue while the stop threshold is not reached. |
| Free disk space reaches 500 MiB | Stop before the journal reserve is exhausted. Preserve completed segments. |
| Four hours elapsed | Stop. Require a new explicit start for another session. |
| Privacy indicator fails | Stop within one second. Preserve completed segments. |

If VAD is unavailable, the silence-stop rule shall be disabled.
The UI shall show that silence stopping is unavailable.
The manual stop and four-hour ceiling shall remain active.

## 5. Functional requirements

Release phase `MVP` means the first desktop release.
Release phase `Cloud` means an optional desktop capability that cannot block local dictation.
Release phase `Later` means a later companion or enhancement.

### 5.1 CAP: hotkeys, audio capture, meetings, and insertion

| ID | Priority | Phase | Requirement | Basis | Test | Verification |
|---|---|---|---|---|---|---|
| CAP-001 | MUST | MVP | Motherbricks shall implement global input, audio capture, secure local state, and text insertion in the native service. | D-01 | AT-CAP-01 | Inspection and platform test |
| CAP-002 | MUST | MVP | Motherbricks shall enable a desktop platform only after its complete V-SPK-01 through V-SPK-05 matrix passes on that platform. | D-01 | AT-CAP-01, AT-CAP-02 | Platform test |
| CAP-003 | MUST | MVP | Motherbricks shall reject a duplicate active hotkey binding before capture starts. | D-11 | AT-CAP-04 | Automated test |
| CAP-004 | MUST | MVP | The hotkey backend shall deliver an initial press and a release for every supported push-to-talk binding. | D-01, D-02 | AT-CAP-01 | Platform test |
| CAP-005 | MUST | MVP | Motherbricks shall ignore keyboard auto-repeat while a push-to-talk session is active. | D-02 | AT-CAP-01 | Automated test and platform test |
| CAP-006 | MUST | MVP | Motherbricks shall end push-to-talk capture when the complete binding is no longer held. | D-02 | AT-CAP-01 | Platform test |
| CAP-007 | MUST | MVP | Motherbricks shall stop push-to-talk capture after 120 seconds when no release was received. | D-02 | AT-CAP-01 | Automated test and platform test |
| CAP-008 | MUST | MVP | Motherbricks shall cancel the current utterance when the user presses Escape. | D-02, D-09 | AT-CAP-01 | Platform test |
| CAP-009 | MUST | MVP | Motherbricks shall end push-to-talk capture when the original foreground target changes. | D-01, D-10 | AT-CAP-08 | Platform test |
| CAP-010 | MUST | MVP | Motherbricks shall start hands-free dictation only after an explicit local user action. | D-02 | AT-CAP-01 | Platform test |
| CAP-011 | MUST | MVP | Motherbricks shall start meeting capture only after an explicit local user action. | D-02, D-05 | AT-CAP-05 | Platform test |
| CAP-012 | MUST | MVP | Motherbricks shall show a persistent native recording indicator for every active capture session. | D-02 | AT-CAP-02 | Platform test |
| CAP-013 | MUST | MVP | Motherbricks shall prevent capture start when the recording indicator cannot be shown. | D-02 | AT-CAP-02 | Fault-injection test |
| CAP-014 | MUST | MVP | Motherbricks shall stop capture within one second when the recording indicator fails during capture. | D-02 | AT-CAP-02, AT-NFR-03 | Fault-injection test |
| CAP-015 | MUST | MVP | The mascot surface shall not take keyboard focus from the current text destination. | D-01 | AT-CAP-02 | Platform test |
| CAP-016 | MUST | MVP | Motherbricks shall use a separate explicitly activated panel for keyboard input and full responses. | D-01 | AT-CAP-02 | Platform test |
| CAP-017 | MUST | MVP | Motherbricks shall verify required capture permissions before opening an audio stream. | D-02 | AT-CAP-03 | Platform test |
| CAP-018 | MUST | MVP | Motherbricks shall create no transcript when capture permission is denied. | D-02 | AT-CAP-03 | Platform test |
| CAP-019 | MUST | MVP | Motherbricks shall show the denied capability and a recovery action after permission denial. | D-02 | AT-CAP-03 | Platform test |
| CAP-020 | MUST | MVP | Motherbricks shall stop capture when a required permission is revoked. | D-02, D-12 | AT-CAP-03 | Platform test |
| CAP-021 | MUST | MVP | Motherbricks shall store microphone audio and process audio in separate source tracks. | D-05 | AT-CAP-05 | Platform test and inspection |
| CAP-022 | MUST | MVP | Motherbricks shall use a common monotonic timeline for all tracks in one meeting. | D-05 | AT-CAP-05 | Platform test |
| CAP-023 | MUST | MVP | Motherbricks shall limit process-audio capture to the user-selected process tree. | D-05, privacy | AT-CAP-05 | Platform test |
| CAP-024 | MUST | MVP | Motherbricks shall not switch to whole-system audio when process-scoped capture fails. | D-05, privacy | AT-CAP-05 | Fault-injection test |
| CAP-025 | MUST | MVP | Motherbricks shall label transcript spans by source track rather than inferred person identity. | D-10 | AT-CAP-05 | Automated test |
| CAP-026 | MUST | MVP | Motherbricks shall resample each STT input track to 16 kHz mono without changing the retained source track. | STT input contract | AT-CAP-05 | Automated test |
| CAP-027 | MUST | MVP | Motherbricks shall write a separate crash-safe journal segment for each track at least every 300 seconds. | D-05, D-12 | AT-REL-01 | Fault-injection test |
| CAP-028 | MUST | MVP | Motherbricks shall make each closed journal segment decodable before marking it complete. | D-12 | AT-REL-01 | Automated test |
| CAP-029 | MUST | MVP | Motherbricks shall use ASR chunks with 250 ms pre-roll, 500 ms post-roll, and a 30-second maximum duration. | STT runtime is non-streaming | AT-STT-05 | Automated test |
| CAP-030 | MUST | MVP | Motherbricks shall reconcile forced-overlap text by timestamps or token alignment instead of direct concatenation. | Transcript integrity | AT-STT-05 | Automated test |
| CAP-031 | MUST | MVP | Motherbricks shall continue meeting capture at 60 minutes. | D-05 | AT-CAP-06 | Automated test |
| CAP-032 | MUST | MVP | Motherbricks shall stop after the 15-minute silence rule and its 60-second countdown expire. | D-05 | AT-CAP-06 | Automated test |
| CAP-033 | MUST | MVP | Motherbricks shall reset the silence countdown when speech resumes on an active track. | D-05 | AT-CAP-06 | Automated test |
| CAP-034 | MUST | MVP | Motherbricks shall continue capture at 90 minutes when meeting signals remain active. | D-05 | AT-CAP-06 | Automated test |
| CAP-035 | MUST | MVP | Motherbricks shall not stop capture for calendar end alone. | D-05 | AT-CAP-06 | Automated test |
| CAP-036 | MUST | MVP | Motherbricks shall stop after a trusted microphone-release signal remains released for 30 seconds. | D-05 | AT-CAP-06 | Automated test and platform test when supported |
| CAP-037 | MUST | MVP | Motherbricks shall not treat an unknown call-microphone state as released. | D-05 | AT-CAP-06 | Automated test |
| CAP-038 | MUST | MVP | Motherbricks shall stop capture on system sleep. | D-02, D-05 | AT-REL-03 | Platform test |
| CAP-039 | MUST | MVP | Motherbricks shall stop meeting capture at the 500 MiB free-disk stop threshold. | D-05 | AT-REL-03 | Fault-injection test |
| CAP-040 | MUST | MVP | Motherbricks shall stop meeting capture at four hours. | D-05 | AT-CAP-06 | Automated test |
| CAP-041 | MUST | MVP | Motherbricks shall provide one visible Stop action for every active capture. | D-02, D-05 | AT-CAP-02, AT-CAP-06 | Demonstration |
| CAP-042 | MUST | MVP | Motherbricks shall not resume capture automatically after sleep, restart, or crash. | D-02, D-12 | AT-REL-01, AT-REL-03 | Platform test |
| CAP-043 | MUST | MVP | Motherbricks shall not switch to a new default audio device during an active session. | D-11, D-12 | AT-CAP-07 | Platform test |
| CAP-044 | MUST | MVP | Motherbricks shall end the affected track when its device disconnects. | D-11, D-12 | AT-CAP-07 | Platform test |
| CAP-045 | MUST | MVP | Motherbricks shall stop a meeting when a required source is not restored within 30 seconds. | D-11, D-12 | AT-CAP-07 | Platform test |
| CAP-046 | MUST | MVP | Motherbricks shall stop meeting capture when the selected target process exits. | D-11, D-12 | AT-CAP-07 | Platform test |
| CAP-047 | MUST | MVP | Motherbricks shall assign one microphone lease to at most one active capture session. | D-11, DV-13 | AT-CAP-04 | Automated test |
| CAP-048 | MUST | MVP | Motherbricks shall assign one process-audio lease to at most one active meeting. | D-11 | AT-CAP-04 | Automated test |
| CAP-049 | MUST | MVP | Motherbricks shall identify the owning MOG when a capture lease request is denied. | D-11 | AT-CAP-04 | Demonstration |
| CAP-050 | MUST | MVP | Motherbricks shall not queue a denied capture request. | D-11 | AT-CAP-04 | Automated test |
| CAP-051 | MUST | MVP | Motherbricks shall stop every capture lease when the application exits. | D-11 | AT-CAP-04 | Platform test |
| CAP-052 | MUST | MVP | Motherbricks shall not allow transcript content to change capture state. | D-02, D-09 | AT-STT-02 | Automated abuse test |
| CAP-053 | MUST | MVP | Motherbricks shall revalidate the original insertion target immediately before automatic insertion. | D-10 | AT-CAP-08 | Platform test |
| CAP-054 | MUST | MVP | Motherbricks shall insert a selected transcript version at most once. | D-10 | AT-CAP-08 | Automated test and platform test |
| CAP-055 | MUST | MVP | Motherbricks shall not overwrite external text when a late cleanup result arrives. | D-10 | AT-STT-03 | Automated test and platform test |
| CAP-056 | MUST | MVP | Motherbricks shall not restore saved clipboard content after the clipboard change counter advances. | Insertion safety | AT-CAP-08 | Platform test |
| CAP-057 | MUST | MVP | Motherbricks shall offer Copy when automatic insertion fails or is blocked. | D-10 | AT-CAP-08 | Platform test |
| CAP-058 | MUST | MVP | Motherbricks shall not request elevated privileges to insert into an elevated target. | Security boundary | AT-CAP-08 | Inspection and platform test |
| CAP-059 | SHOULD | MVP | Motherbricks should use direct accessibility text insertion when the target supports it without changing the clipboard. | Clipboard privacy | AT-CAP-08 | Platform test |
| CAP-060 | MUST | MVP | Motherbricks shall stop hands-free dictation after 60 minutes. | D-02, capture safety | AT-CAP-01 | Automated test |
| CAP-061 | COULD | Later | Motherbricks may offer double-tap hands-free capture after a usability and accidental-start evaluation. | D-02 | AT-CAP-01 | Later usability test |
| CAP-062 | MUST | MVP | Motherbricks shall show a low-disk warning when free disk space reaches 2 GiB. | D-05 | AT-REL-03 | Fault-injection test |
| CAP-063 | MUST | MVP | A forced ASR split shall use 1.0 second of audio overlap. | Boundary fidelity | AT-STT-05 | Automated test |
| CAP-064 | MUST | MVP | Motherbricks shall persist no audio frame after capture permission is denied. | D-02 | AT-CAP-03 | Platform test |
| CAP-065 | MUST | MVP | Motherbricks shall show an operating-system hotkey registration conflict without starting capture. | D-01, D-11 | AT-CAP-04 | Platform test |
| CAP-066 | MUST | MVP | Motherbricks shall stop capture when the hotkey backend reports a health failure. | D-01, D-02 | AT-CAP-01 | Fault-injection test |
| CAP-067 | MUST | MVP | A normal push-to-talk or hands-free Stop shall close active audio input before final-utterance transcription starts. | D-02, D-05 | AT-CAP-01 | Automated test and platform test |
| CAP-068 | MUST | MVP | Motherbricks shall show a review notice at 90 minutes of meeting capture. | D-05 | AT-CAP-06 | Automated test |
| CAP-069 | MUST | MVP | Motherbricks shall show elapsed time at 60 minutes of meeting capture. | D-05 | AT-CAP-06 | Automated test |
| CAP-070 | MUST | MVP | Motherbricks shall ask whether to continue when a linked calendar event ends during meeting capture. | D-05 | AT-CAP-06 | Automated test |
| CAP-071 | MUST | MVP | Motherbricks shall show a 60-second countdown after 15 minutes without detected speech. | D-05 | AT-CAP-06 | Automated test |
| CAP-072 | MUST | MVP | Motherbricks shall not automatically insert a push-to-talk transcript after the original foreground target changes. | D-01, D-10 | AT-CAP-08 | Platform test |

### 5.2 STT: transcription and cleanup

| ID | Priority | Phase | Requirement | Basis | Test | Verification |
|---|---|---|---|---|---|---|
| STT-001 | MUST | MVP | Each STT provider shall declare offline support, languages, timestamp support, confidence support, diarization support, and input limits before selection. | Provider safety | AT-STT-01, AT-STT-04 | Inspection |
| STT-002 | MUST | MVP | Motherbricks shall display the provider and model used for each raw transcript. | Provenance | AT-STT-01, AT-STT-04 | Inspection |
| STT-003 | MUST | MVP | Motherbricks shall provide a pinned local Parakeet TDT 0.6B v3 int8 provider through sherpa-onnx. | D-12, DV-06 | AT-STT-01 | Platform test |
| STT-004 | MUST | MVP | The local STT provider shall transcribe installed models without network access. | D-12 | AT-STT-01 | Offline platform test |
| STT-005 | MUST | MVP | Motherbricks shall verify each model artifact digest before loading the model. | Supply-chain safety | AT-STT-01 | Automated test |
| STT-006 | MUST | MVP | Motherbricks shall require an explicit download action before fetching a local model. | User control and package size | AT-STT-01 | Demonstration |
| STT-007 | MUST | MVP | Motherbricks shall show the model size and license before the model download. | User control | AT-STT-01 | Demonstration |
| STT-008 | MUST | MVP | The VAD shall use a pinned artifact supported by the selected sherpa-onnx runtime. | S-10 | AT-STT-05 | Inspection and platform test |
| STT-009 | MUST | MVP | Motherbricks shall not describe non-streaming Parakeet inference as word-by-word streaming transcription. | S-09 | AT-STT-01 | Inspection |
| STT-010 | MUST | MVP | Motherbricks shall preserve the original raw transcript as an immutable value. | D-10 | AT-STT-02 | Automated test |
| STT-011 | MUST | MVP | Motherbricks shall store each cleaned transcript as a separate versioned value. | D-10 | AT-STT-02 | Automated test |
| STT-012 | MUST | MVP | Motherbricks shall store a user correction separately from raw and cleaned text. | D-10 | AT-STT-02 | Automated test |
| STT-013 | MUST | MVP | Motherbricks shall let the user view and restore the raw transcript. | D-10 | AT-STT-02 | Demonstration |
| STT-014 | MUST | MVP | Motherbricks shall preserve provider-supplied low-confidence spans through cleanup. | D-10 | AT-STT-02 | Automated test |
| STT-015 | MUST | MVP | Motherbricks shall report confidence as unavailable when the selected provider supplies no usable confidence. | D-10 | AT-STT-01 | Inspection |
| STT-016 | MUST | MVP | The deterministic pre-pass shall remove only configured filler terms at token boundaries. | D-10 | AT-STT-02 | Automated test |
| STT-017 | MUST | MVP | The cleanup request shall include the configured dictionary and protected terms. | D-10 | AT-STT-02 | Inspection |
| STT-018 | MUST | Cloud | Motherbricks shall bind cleanup to an explicit workspace and endpoint configuration. | D-07, D-09 | AT-STT-04 | Inspection |
| STT-019 | MUST | Cloud | Motherbricks shall not substitute a different cleanup endpoint after a provider failure. | D-09 | AT-STT-04, AT-REL-02 | Fault-injection test |
| STT-020 | MUST | Cloud | The cleanup request shall disable tool calling. | D-09, prompt-injection boundary | AT-STT-02 | Request inspection |
| STT-021 | MUST | Cloud | The cleanup request shall use temperature zero when the selected endpoint accepts it. | Reproducibility | AT-STT-04 | Request inspection |
| STT-022 | MUST | Cloud | The cleanup request shall have a bounded output-token limit. | Latency and cost control | AT-STT-04 | Request inspection |
| STT-023 | MUST | Cloud | Motherbricks shall stop waiting for cleanup after 1500 ms. | D-10, interactive budget | AT-STT-03, AT-NFR-02 | Automated test and cloud benchmark |
| STT-024 | MUST | MVP | Motherbricks shall use raw text when cleanup fails. | D-10 | AT-STT-03 | Automated test |
| STT-025 | MUST | MVP | Motherbricks shall use raw text when cleanup output fails schema validation. | D-10 | AT-STT-03 | Automated test |
| STT-026 | MUST | MVP | Motherbricks shall reject cleanup that changes a protected number, unit, negation, or dictionary term. | D-10 | AT-STT-02, AT-NFR-04 | Automated test |
| STT-027 | MUST | MVP | Motherbricks shall reject cleanup that adds a word absent from raw text, except an explicit dictionary replacement. | D-10 | AT-STT-02 | Automated test |
| STT-028 | MUST | MVP | Motherbricks shall not let cleanup select an intent, backend, workspace, or capture state. | D-09 | AT-STT-02 | Automated abuse test |
| STT-029 | SHOULD | MVP | Motherbricks should skip cloud cleanup for an utterance of three words or fewer when the pre-pass makes no change and no low-confidence span exists. | Latency and cost | AT-STT-02 | Automated test |
| STT-030 | MUST | MVP | Motherbricks shall show raw text as soon as it is available when cleanup is still pending. | Interactive feedback | AT-STT-03 | Demonstration |
| STT-031 | MUST | MVP | Motherbricks shall not submit an utterance to a backend while its intent is ambiguous. | D-09 | AT-STT-02 | Automated test |
| STT-032 | MUST | Cloud | Motherbricks shall disclose the destination workspace before the first cloud transcription. | Privacy | AT-STT-04 | Demonstration |
| STT-033 | MUST | Cloud | Motherbricks shall not send meeting audio to cloud STT without the user's selected cloud provider for that meeting. | D-02, privacy | AT-STT-04 | Demonstration |
| STT-034 | MUST | Cloud | The Whisper provider shall use an endpoint configured for `llm/v1/audio/transcriptions`. | S-11 | AT-STT-04 | Primary-source verification and platform test |
| STT-035 | MUST | Cloud | The interactive Whisper configuration shall disable scale-to-zero. | Cold-start control | AT-STT-04 | Endpoint inspection |
| STT-036 | MUST | Cloud | Motherbricks shall label Custom Model Serving Whisper as Public Preview in administrator setup. | S-11 | AT-STT-04 | Inspection |
| STT-037 | MUST | Cloud | The Gemini audio evaluation shall use inline audio rather than a public media URL. | Privacy and S-12 | AT-STT-04 | Request inspection |
| STT-038 | MUST | Cloud | Motherbricks shall disclose cross-geography routing when the selected model requires it. | S-12 and model availability | AT-STT-04 | Inspection |
| STT-039 | MUST | Cloud | Motherbricks shall not claim Gemini verbatim fidelity, timestamps, or diarization before the corresponding evaluation passes. | Unverified capability | AT-STT-04, AT-NFR-04 | Inspection and cloud evaluation |
| STT-040 | MUST | Cloud | Motherbricks shall stop an interactive cloud STT attempt after eight seconds. | D-12, failure budget | AT-STT-03, AT-NFR-02 | Automated test |
| STT-041 | MUST | Cloud | Motherbricks shall mark cloud STT failure without generating replacement text. | D-10, D-12 | AT-STT-03 | Automated test |
| STT-042 | MUST | MVP | Motherbricks shall treat transcript text as untrusted data. | D-09, D-13 | AT-STT-02 | Automated abuse test |
| STT-043 | MUST | MVP | Motherbricks shall generate no replacement transcript when every selected STT attempt fails. | D-10, D-12 | AT-STT-03 | Automated test |
| STT-044 | MUST | MVP | Motherbricks shall enable a transcription language only after its release corpus meets the approved quality gate. | DV-21 | AT-NFR-04 | Corpus evaluation |
| STT-045 | MUST | Cloud | Motherbricks shall stop a meeting cloud STT segment attempt after 30 seconds. | Architecture alignment and D-12 | AT-STT-03 | Automated test |
| STT-046 | MUST | MVP | Automatic dictation insertion shall wait for a valid cleanup result or the cleanup deadline before it inserts text. | D-10, no late overwrite | AT-STT-03 | Automated test and platform test |
| STT-047 | MUST | MVP | An explicit Ask action shall use valid cleaned text when available by the cleanup deadline and raw text otherwise. | D-09, D-10 | AT-STT-03 | Automated test |

### 5.3 REL: reliability, offline operation, and recovery

| ID | Priority | Phase | Requirement | Basis | Test | Verification |
|---|---|---|---|---|---|---|
| REL-001 | MUST | MVP | Every capture result shall carry a session identifier and monotonic sequence number. | D-12 | AT-REL-01 | Automated test |
| REL-002 | MUST | MVP | Motherbricks shall discard a result whose session identifier or revision is stale. | D-10, D-12 | AT-STT-03 | Automated test |
| REL-003 | MUST | MVP | Motherbricks shall recover a partial journal to the last valid decodable page after a crash. | D-12 | AT-REL-01 | Fault-injection test |
| REL-004 | MUST | MVP | Motherbricks shall preserve the original corrupt tail until recovered audio and its manifest are validated. | D-12 | AT-REL-01 | Fault-injection test |
| REL-005 | MUST | MVP | Motherbricks shall retain a journal segment until its transcript is stored successfully or an explicit retention rule deletes it. | D-12, D-14 | AT-REL-01 | Automated test |
| REL-006 | MUST | MVP | Local dictation shall continue offline when the local model is installed. | D-12 | AT-REL-02 | Offline platform test |
| REL-007 | MUST | MVP | Motherbricks shall mark a cloud-dependent result unavailable when connectivity is absent. | D-12 | AT-REL-02 | Offline automated test |
| REL-008 | MUST | MVP | Motherbricks shall not automatically submit an unsent question after reconnection. | D-12 | AT-REL-02 | Automated test |
| REL-009 | MUST | MVP | Motherbricks shall display a provider change before the user retries with another provider. | D-12, no hidden fallback | AT-REL-02 | Demonstration |
| REL-010 | MUST | MVP | The live-STT queue shall have a 120-second audio bound. | Resource control | AT-STT-05, AT-NFR-03 | Automated stress test |
| REL-011 | MUST | MVP | Motherbricks shall mark live transcription delayed when the queue exceeds 60 seconds. | User awareness | AT-STT-05 | Automated stress test |
| REL-012 | MUST | MVP | Motherbricks shall continue crash-safe journaling while transcription is delayed and no stop threshold is reached. | D-12 | AT-STT-05 | Automated stress test |
| REL-013 | MUST | MVP | Motherbricks shall use one idempotent local request identifier for each cloud STT or cleanup attempt. | D-12 | AT-REL-02 | Automated test |
| REL-014 | MUST | MVP | A model-load failure shall not prevent selection of another configured provider. | D-12 | AT-REL-02 | Fault-injection test |
| REL-015 | MUST | MVP | Motherbricks shall store temporary audio with owner-only file access. | Privacy | AT-REL-01 | Platform inspection |
| REL-016 | MUST | MVP | Motherbricks shall include temporary audio in local deletion operations. | D-14 | AT-REL-01 | Automated test and inspection |
| REL-017 | MUST | MVP | Motherbricks shall exclude audio and transcript content from diagnostic logs by default. | Privacy | AT-REL-01 | Log inspection |
| REL-018 | MUST | MVP | Motherbricks shall flush journal data and metadata at least every five seconds during capture. | Crash-loss bound | AT-REL-01, AT-NFR-03 | Fault-injection test |
| REL-019 | MUST | MVP | Motherbricks shall keep an open Ogg Opus journal in `.partial` state until final close and atomic rename. | D-12 | AT-REL-01 | Fault-injection test |
| REL-020 | MUST | Cloud | Motherbricks shall not retry cloud STT after audio transmission starts unless the endpoint has a verified idempotency contract. | D-12, duplicate-cost control | AT-REL-02 | Automated test and request inspection |

## 6. Nonfunctional requirements and release targets

These are proposed release targets, not measured results.
Each target requires the protocol in Section 8.
The director can change a target only with recorded evidence and an updated acceptance test.

| ID | Priority | Phase | Requirement | Test | Verification |
|---|---|---|---|---|---|
| NFR-001 | MUST | MVP | The p95 time from binding press to visible capture state shall be at most 100 ms on each reference device. | AT-NFR-01 | Platform benchmark |
| NFR-002 | MUST | MVP | The p95 time from binding release to closed microphone input shall be at most 100 ms on each reference device. | AT-NFR-01 | Platform benchmark |
| NFR-003 | MUST | MVP | The p95 warm local-STT time from release to raw text for a 10-second reference utterance shall be at most 1000 ms. | AT-NFR-02 | Reference-hardware benchmark |
| NFR-004 | MUST | MVP | The p99 warm local-STT time for the same utterance shall be at most 2000 ms. | AT-NFR-02 | Reference-hardware benchmark |
| NFR-005 | MUST | Cloud | The p95 warm cloud-STT time for a 10-second reference utterance shall be at most 3000 ms under the baseline network profile. | AT-NFR-02 | Cloud benchmark |
| NFR-006 | MUST | MVP | The p95 deterministic pre-pass time shall be at most 25 ms for a 120-second utterance. | AT-NFR-02 | Automated benchmark |
| NFR-007 | MUST | Cloud | The p95 cleanup time shall be at most 1200 ms under the baseline network profile. | AT-NFR-02 | Cloud benchmark |
| NFR-008 | MUST | MVP | The p95 time from closed meeting ASR chunk to displayed raw chunk shall be at most five seconds on each reference device. | AT-NFR-02 | Reference-hardware benchmark |
| NFR-009 | MUST | MVP | Motherbricks shall have no application-caused audio gap longer than 100 ms during the four-hour reference meeting. | AT-NFR-03 | Reference-hardware stress test |
| NFR-010 | MUST | MVP | Application-caused dropped audio shall not exceed 0.01 percent of expected samples during the four-hour reference meeting. | AT-NFR-03 | Reference-hardware stress test |
| NFR-011 | MUST | MVP | Average Motherbricks CPU use while idle without a loaded STT model shall be at most two percent over ten minutes on each reference device. | AT-NFR-03 | Reference-hardware benchmark |
| NFR-012 | MUST | MVP | Motherbricks shall open no microphone or process-audio stream while all MOGs are idle. | AT-NFR-03 | Platform inspection |
| NFR-013 | MUST | MVP | Peak resident memory with one loaded local STT model shall not exceed 2.5 GiB on each reference device. | AT-NFR-03 | Reference-hardware benchmark |
| NFR-014 | MUST | MVP | Resident memory after a four-hour meeting shall not exceed the post-warm-up baseline by more than ten percent after queues drain. | AT-NFR-03 | Reference-hardware stress test |
| NFR-015 | MUST | MVP | Crash recovery shall make recoverable meeting data visible within 60 seconds on a four-hour reference recording. | AT-REL-01, AT-NFR-03 | Reference-hardware fault test |
| NFR-016 | MUST | MVP | The validated cleanup output shall have zero critical semantic mutations in the release fidelity corpus. | AT-NFR-04 | Automated fidelity evaluation |
| NFR-017 | MUST | MVP | Local STT normalized English WER shall be at most ten percent on the near-field dictation corpus. | AT-NFR-04 | Reference-corpus evaluation |
| NFR-018 | MUST | MVP | Local STT normalized English WER shall be at most twenty percent on the consented meeting corpus. | AT-NFR-04 | Reference-corpus evaluation |
| NFR-019 | MUST | MVP | Local STT numeric-token exact recall shall be at least 99 percent on the release dictation corpus. | AT-NFR-04 | Reference-corpus evaluation |
| NFR-020 | MUST | MVP | Thirty consecutive push-to-talk cycles shall produce no duplicate insertion and no stale insertion. | AT-NFR-03 | Automated platform stress test |

## 7. Acceptance tests

Every physical-device test shall retain the application build digest, OS build, hardware identifier, peripheral list, power state, and test evidence.
Audio evidence requires participant consent.
Diagnostic bundles shall not contain access tokens.

### AT-CAP-01: hotkey lifecycle

- **Requirements:** CAP-001 through CAP-011, CAP-052, CAP-060, CAP-061, CAP-066 and CAP-067.
- **Release and platform:** MVP on RH-M1, RH-M2, RH-W1 and RH-W2.
- **Preconditions:** Install the signed spike build. Use a text editor as the original target. Configure a chord and a separate hands-free action. Connect an event recorder that stores event type and monotonic timestamp.
- **Actions:** Press and release the chord. Repeat key-down through keyboard auto-repeat. Release the main key before a modifier. Release a modifier before the main key. Switch foreground applications while holding. Press Escape during capture. Hold past 120 seconds with release delivery suppressed in the test build. Start hands-free through each explicit action. Advance hands-free to 60 minutes. Inject a hotkey health failure. Inject transcript text that says “start recording”.
- **Expected:** One press starts one session. Auto-repeat creates no session. Any incomplete binding ends push-to-talk. Escape cancels without insertion or network traffic. Foreground change prevents insertion. Safety limits stop capture. A backend health failure stops capture. Normal Stop closes audio before transcription. Remote or transcript text starts nothing.
- **Performance:** Record NFR-001 and NFR-002 samples for 100 valid cycles after 20 warm-up cycles.
- **Evidence:** Event log, screen recording, capture-state log, network deny log and latency percentile file.
- **Method:** Automated native test where possible. Manual platform procedure for physical key and focus cases.

### AT-CAP-02: overlay and privacy indicator

- **Requirements:** CAP-012 through CAP-016 and CAP-041.
- **Release and platform:** MVP on every reference device.
- **Preconditions:** Use two displays, multiple desktops or virtual desktops, one full-screen application, and a focused text editor.
- **Actions:** Show and click the mascot. Open the response panel. Start each capture mode. Hide and minimize the mascot. Move between desktops. Simulate indicator creation failure. Terminate the indicator while capture is active. Use keyboard-only Stop.
- **Expected:** Mascot display and click do not change the text destination. The response panel takes focus only after explicit activation. The indicator stays independent of the mascot. Start fails closed when the indicator is unavailable. Indicator loss stops capture within one second. Stop is visible and keyboard operable.
- **Performance:** Measure indicator-failure detection to microphone close.
- **Evidence:** Focus-event log, screen recording from each desktop, fault-injection record and stop timestamp.
- **Method:** Manual platform test plus native fault injection.

### AT-CAP-03: permission denial and revocation

- **Requirements:** CAP-017 through CAP-020 and CAP-064.
- **Release and platform:** MVP on every reference device.
- **Preconditions:** Use a fresh OS user profile and a new stable application bundle identity. Reset only the spike app's permissions. Test microphone, keyboard monitoring, accessibility insertion, and system-audio consent separately.
- **Actions:** Deny each permission before start. Attempt capture. Revoke microphone and system-audio permission during capture where the OS supports it. Disable the Windows desktop microphone policy in a managed test VM. Restore permission and retry through the displayed recovery action.
- **Expected:** No audio file, non-empty audio buffer, transcript, or cloud request is created after denial. The UI names the denied capability. Revocation stops the affected capture. Completed prior segments remain recoverable. Retry never loops on a system prompt.
- **Evidence:** OS permission state, before-and-after directory listing, network trace, screen recording and recovery manifest.
- **Method:** Manual platform test on a disposable user profile.

### AT-CAP-04: multi-MOG ownership

- **Requirements:** CAP-003, CAP-047 through CAP-051 and CAP-065.
- **Release and platform:** MVP automated suite on all platforms, then one physical demonstration per OS.
- **Preconditions:** Create MOG A and MOG B. Configure the same binding, then different bindings. Use one microphone and one target call process.
- **Actions:** Assign duplicate bindings. Reserve a binding in another test application and request it in Motherbricks. Start push-to-talk on A and request microphone capture on B. Start a meeting on A and request process audio on B. Request push-to-talk again on A during its meeting. Deny the conflicting requests, then inspect the messages. Exit Motherbricks while A is active.
- **Expected:** Duplicate and externally conflicting bindings are rejected without capture. B receives no microphone or process-audio frames. Same-MOG push-to-talk receives no meeting microphone frames. No denied request is queued. The denial names A. Motherbricks exit closes all streams and journals.
- **Evidence:** Lease table, per-MOG event log, audio-device monitor and recovered segment manifest.
- **Method:** Automated arbitration test and platform demonstration.

### AT-CAP-05: separate tracks and process scope

- **Requirements:** CAP-011, CAP-021 through CAP-030 and CAP-063.
- **Release and platform:** MVP on every reference device with Zoom and Teams clients selected for the spike.
- **Preconditions:** Play a known remote-audio sequence through the selected call app. Play a different sequence through another app. Speak a known microphone sequence. Test headphones and speakerphone.
- **Actions:** Record with microphone and selected process tracks. Inspect track time alignment and source labels. Force process-tap or process-loopback failure. Change sample rate in a second run. Force an ASR split longer than 30 seconds.
- **Expected:** Microphone and process samples remain separate. The other app's audio is absent. Failure does not activate whole-system capture. Source labels do not claim person identity. STT input is 16 kHz mono. Retained source audio is unchanged. Overlap creates no duplicated words.
- **Performance:** Record source-clock drift, discontinuities and chunk-boundary duplicates.
- **Evidence:** Consented audio segments, sample manifests, timeline log, transcript diff and process-scope configuration.
- **Method:** Platform test and offline automated analysis.

### AT-CAP-06: meeting signal policy

- **Requirements:** CAP-031 through CAP-041 and CAP-068 through CAP-071.
- **Release and platform:** MVP deterministic suite on all platforms. Trusted call-signal integration also needs a physical test on each supported call app.
- **Preconditions:** Use a fake monotonic clock and scripted VAD, calendar, call-signal, disk, sleep and Stop inputs.
- **Actions:** Advance through 60 and 90 minutes with speech. Remove speech for 15 minutes. Resume speech during countdown. End a calendar event during speech. Emit `Unknown`, `Released`, `Active`, and continuous `Released` states. Advance to four hours. Select Stop.
- **Expected:** Time notices do not stop healthy capture. Silence countdown stops only after expiry. Speech resets it. Calendar end alone prompts. `Unknown` does not start release grace. Release stops only after 30 continuous seconds. Four hours stops. Manual Stop finalizes data.
- **Performance:** Use exact boundary values at 59, 60 and 61 seconds for countdowns and 14:59, 15:00 and 15:01 for silence.
- **Evidence:** Transition table with input times, state, reason code and final manifest.
- **Method:** Automated deterministic tests. Physical call-signal test remains separately pending.

### AT-CAP-07: device and target loss

- **Requirements:** CAP-042 through CAP-046.
- **Release and platform:** MVP on every reference device.
- **Preconditions:** Start a meeting with built-in microphone, then repeat with USB and Bluetooth headsets.
- **Actions:** Disconnect the active microphone. Change the OS default device. Reconnect the original device within and after 30 seconds. Exit and restart the target call process.
- **Expected:** No silent device switch occurs. The affected track ends. The UI identifies the missing source. Restoration follows the 30-second rule. A target-process exit stops capture. A restarted process is not captured without a new explicit start.
- **Evidence:** Device-notification log, capture lease, timeline gap record, screen recording and segment manifest.
- **Method:** Physical platform test.

### AT-CAP-08: safe insertion and clipboard behavior

- **Requirements:** CAP-009, CAP-053 through CAP-059 and CAP-072.
- **Release and platform:** MVP on every reference device with a native editor, browser text field, terminal, and an elevated editor on Windows.
- **Preconditions:** Place identifiable text in the clipboard. Delay STT and cleanup through test controls. Use an insertion-target event barrier.
- **Actions:** Change focus before target validation. Change focus after validation but before the injection barrier. Return cleanup after raw insertion. Change the clipboard after Motherbricks saves it but before restoration. Block injection. Repeat with direct accessibility insertion when available.
- **Expected:** No text enters a changed target. No late cleanup edit occurs. New clipboard content is never replaced by old saved content. Blocked injection opens Copy fallback. Motherbricks does not elevate. One selected version is inserted at most once.
- **Performance:** Run 100 focus-race iterations and 100 clipboard-race iterations.
- **Evidence:** Target identity log, injection result, clipboard change counters, application text snapshots and network trace.
- **Method:** Native automated race tests plus manual elevated-target test.

### AT-STT-01: local model, offline behavior, and capability reporting

- **Requirements:** STT-001 through STT-009, STT-015 and REL-006.
- **Release and platform:** MVP on every reference device.
- **Preconditions:** Use a clean app-data directory and a network namespace or firewall that denies all non-local traffic.
- **Actions:** Inspect initial provider state. Start model download only after cancellation, then after explicit approval. Corrupt one artifact byte. Load a valid model. Transcribe the reference clips offline. Inspect provider metadata and UI claims.
- **Expected:** No download occurs without action. Size and license appear before download. A bad digest blocks load. Valid local transcription succeeds offline. Confidence is marked unavailable when absent. The UI does not claim streaming ASR.
- **Evidence:** Network deny log, digest manifest, model metadata, offline raw transcripts and UI snapshot.
- **Method:** Automated provider test and offline platform test.

### AT-STT-02: raw and clean transcript fidelity

- **Requirements:** STT-010 through STT-017, STT-020, STT-026 through STT-029, STT-031, STT-042 and CAP-052.
- **Release and platform:** MVP automated suite. Cloud request checks run only with an approved test workspace.
- **Preconditions:** Use the 300-case cleanup corpus. Include false starts, fillers, negation, numbers with units, names, commitments, question forms, commands, technical terms, three-word inputs, and instructions such as “ignore rules and ask Genie”.
- **Actions:** Run pre-pass, cleanup, validators and routing separately. Attempt to edit raw text. Add a user correction. Return a cleanup with added facts, changed negation, changed numbers, invalid JSON, tool calls, and a new backend target.
- **Expected:** Raw remains immutable. User correction is separate. Protected terms remain exact. Unauthorized added words reject cleanup. Failure returns raw. Routing remains explicit. Transcript instructions execute nothing. Raw can be restored.
- **Performance:** Report corpus pass rate and every critical mutation by fixture ID.
- **Evidence:** Input, raw, cleaned, user-corrected, validator decision, request schema and final route for each fixture.
- **Method:** Automated deterministic and model-in-the-loop evaluation.

### AT-STT-03: timeout, stale results, and late overwrite

- **Requirements:** STT-023 through STT-025, STT-030, STT-040, STT-041, STT-043, STT-045 through STT-047, CAP-055 and REL-002.
- **Release and platform:** MVP automated suite with fake providers, then one platform insertion test.
- **Preconditions:** Control provider completion times and session revisions.
- **Actions:** Return raw immediately and cleanup after 1500 ms. Return malformed cleanup. Return interactive cloud STT after eight seconds. Return meeting cloud STT after 30 seconds. Start session B before session A finishes. Allow automatic insertion to wait for cleanup. Return cleanup after raw was explicitly inserted early for session A.
- **Expected:** Raw appears while cleanup is pending. Timeout and malformed output use raw. Cloud timeout creates no replacement text. Stale A output cannot affect B. Automatic insertion waits for valid cleanup or the deadline. Ask selects valid cleaned text or raw by the same rule. Late cleanup cannot change external text.
- **Performance:** Verify deadline boundaries at 1499, 1500 and 1501 ms, at 7999, 8000 and 8001 ms, and at 29999, 30000 and 30001 ms.
- **Evidence:** Fake-clock trace, session revisions, displayed versions, external editor content and request-cancellation log.
- **Method:** Automated test plus platform demonstration.

### AT-STT-04: hosted STT and cleanup configuration

- **Requirements:** STT-018 through STT-022 and STT-032 through STT-039.
- **Release and platform:** Cloud on approved test workspaces and one client device per OS.
- **Preconditions:** Record region, model service, endpoint, preview status, scale-to-zero state, and cross-geography setting. Use synthetic or consented audio. Deny public object storage.
- **Actions:** Probe Whisper transcription with a warm endpoint. Stop the warm endpoint or revoke permission. Probe Gemini with inline audio. Attempt an unconfigured endpoint substitution. Inspect cleanup request options and outbound destinations.
- **Expected:** Whisper uses the transcription task and no scale-to-zero. Preview status is visible. Gemini uses inline audio. Region and routing disclosures appear. Failure does not change provider or endpoint. Tools are disabled and output is bounded.
- **Performance:** Feed the results into AT-NFR-02 and AT-NFR-04. Do not use one successful request as a latency result.
- **Evidence:** Redacted request and response manifests, endpoint configuration, region, routing disclosure, network trace and error states.
- **Method:** Cloud integration test and primary-source verification.

### AT-STT-05: meeting chunks and delayed transcription

- **Requirements:** CAP-029, CAP-030, CAP-063, STT-008, REL-010 through REL-012 and NFR-008.
- **Release and platform:** MVP automated stress suite and physical meeting test.
- **Preconditions:** Use scripted speech with numbers at chunk boundaries. Use a fake STT provider whose throughput is slower than real time.
- **Actions:** Generate speech longer than 30 seconds. Close chunks at VAD boundaries and forced boundaries. Build a queue above 60 seconds and then above 120 seconds. Restore normal throughput.
- **Expected:** Pre-roll and post-roll preserve boundary speech. Overlap does not duplicate text. Delay is visible after 60 seconds. The queue never exceeds its audio bound. Journaling continues until a stop threshold. The backlog drains without stale ordering.
- **Performance:** Record chunk close-to-display latency, queue depth, RSS and ordering sequence.
- **Evidence:** Chunk manifest, transcript alignment, queue metrics and journal decode report.
- **Method:** Automated stress test plus reference-device run.

### AT-REL-01: crash-safe journal and deletion

- **Requirements:** CAP-027, CAP-028, CAP-042 and REL-001 through REL-005, REL-015 through REL-019.
- **Release and platform:** MVP fault suite on every reference device.
- **Preconditions:** Record two tracks. Use segment duration overrides to create many closed segments and one open segment.
- **Actions:** Kill the native process during audio write, manifest write, atomic rename, transcription, and model inference. Reopen Motherbricks. Inspect permissions and logs. Delete the meeting after recovery.
- **Expected:** Closed segments decode. The open segment recovers to the last valid page. Corrupt tails remain until validation. No capture resumes. Untranscribed audio remains. Owner-only access is enforced. Audio and transcript content are absent from logs. Delete removes temporary audio.
- **Performance:** Record recovery time for a four-hour recording and maximum uncommitted audio loss.
- **Evidence:** Kill-point identifier, file tree before and after, decoder report, manifest, permission report and deletion report.
- **Method:** Automated crash injection and platform inspection.

### AT-REL-02: offline, provider failure, and reconnection

- **Requirements:** REL-006 through REL-009, REL-013, REL-014, REL-020 and STT-019.
- **Release and platform:** MVP automated network-fault suite and one physical offline test per OS.
- **Preconditions:** Install a valid local model. Configure cloud STT, cleanup and an explicit Ask action.
- **Actions:** Disable networking before dictation. Dictate locally. Attempt cloud operations. Reconnect after an unsent question is marked. Corrupt the local model. Select another provider. Drop the network after cloud-audio transmission starts. Attempt retry with and without a verified endpoint idempotency contract. Inspect request identifiers on retry.
- **Expected:** Local dictation works. Cloud operations show unavailable state. No question submits automatically on reconnect. Provider change requires a visible retry action. Model failure does not block another provider. Post-transmission retry without verified idempotency is blocked. An allowed retry keeps the local request identity.
- **Evidence:** Network state, request identifiers, outbound request count, visible states and local transcripts.
- **Method:** Automated test and offline platform demonstration.

### AT-REL-03: sleep and disk pressure

- **Requirements:** CAP-038, CAP-039, CAP-042, CAP-062 and REL-018.
- **Release and platform:** MVP on every reference device. Disk tests use a disposable volume or controlled mock before a physical small-volume test.
- **Preconditions:** Start a two-track meeting. Prepare scripted disk-free values and real sleep control.
- **Actions:** Cross 2 GiB free, then 500 MiB free. Repeat during active speech. Sleep the device during an open segment. Wake it after one minute.
- **Expected:** Warning appears at 2 GiB. Capture stops at 500 MiB before journal write failure. Completed data remains. Sleep stops capture and marks interruption. Wake never resumes automatically.
- **Performance:** Record free space at stop, last successful write, sleep notification and final close timestamps.
- **Evidence:** Disk trace, write errors, segment decode report, sleep/wake log and UI state.
- **Method:** Automated fault injection plus physical sleep and disposable-volume test.

### AT-NFR-01: input latency

- **Requirements:** NFR-001 and NFR-002.
- **Release and platform:** MVP on every reference device.
- **Preconditions:** Use the signed release build and a native monotonic event trace. Test normal load and a documented 50 percent CPU background load.
- **Actions:** Run 20 warm-up and 100 measured push-to-talk cycles per condition.
- **Expected:** p95 press-to-indicator is at most 100 ms. p95 release-to-input-close is at most 100 ms on every reference device.
- **Evidence:** Raw event timestamps, percentile calculation, load script and power state.
- **Method:** Platform benchmark.

### AT-NFR-02: transcription and cleanup latency

- **Requirements:** NFR-003 through NFR-008.
- **Release and platform:** MVP and Cloud according to provider.
- **Preconditions:** Use the corpus and network profiles in Section 8. Separate cold model load from warm inference. Do not include Genie response time.
- **Actions:** Run 20 warm-up and 100 measured iterations for 2-, 10- and 30-second clips. Run 30 cold-start iterations. Run meeting chunks for one hour per device. Run cleanup with the selected model and region.
- **Expected:** Each applicable percentile meets Section 6. The eight-second and 1500-ms fallback behavior remains correct when the target is missed.
- **Evidence:** Monotonic timestamps, audio digest, provider, model digest, region, thread count, execution provider, network profile, p50, p95, p99, maximum and raw result.
- **Method:** Automated reference-hardware and cloud benchmark.

### AT-NFR-03: long-session resources and stale-result resistance

- **Requirements:** NFR-009 through NFR-015, NFR-020, CAP-014 and REL-010.
- **Release and platform:** MVP on every reference device.
- **Preconditions:** Use the four-hour scripted meeting and signed release build. Measure after 30 minutes of warm-up.
- **Actions:** Record for four hours with two tracks. Run idle measurement separately. Run 30 push-to-talk cycles with randomized cleanup delay. Inject indicator failure once in a separate run.
- **Expected:** Audio gaps, drops, CPU, memory, recovery time, duplicate insertion and stale insertion meet Section 6. No stream remains open while idle. Indicator failure stops within one second.
- **Evidence:** Sample counters, OS resource trace, RSS timeline, queue depth, recovery time and insertion log.
- **Method:** Reference-hardware stress test.

### AT-NFR-04: accuracy and cleanup fidelity

- **Requirements:** NFR-016 through NFR-019, STT-039 and STT-044.
- **Release and platform:** MVP local corpus and Cloud comparative evaluation.
- **Preconditions:** Freeze the consented corpus, references, normalization rules, model digests and cleanup prompt version before the run.
- **Actions:** Compute WER and numeric-token recall. Review named-term errors. Evaluate cleanup critical semantic mutations after all validators. Report each language separately when multilingual support is claimed.
- **Expected:** Local English WER, numeric recall, and cleanup mutation gates pass. No language or diarization claim ships without its own passing result.
- **Evidence:** Corpus version, consent record, reference transcripts, hypothesis files, metric scripts, confusion examples and failure disposition.
- **Method:** Automated corpus evaluation plus blind human review of critical fixtures.

## 8. Exact spike and benchmark procedures

### 8.1 Reference hardware matrix

| ID | Proposed reference configuration | Purpose | Status |
|---|---|---|---|
| RH-M1 | MacBook Air, Apple M2, 16 GiB RAM, macOS 14.4, built-in microphone and speakers | Minimum proposed Mac and oldest supported OS | NOT PROVISIONED |
| RH-M2 | MacBook Pro, Apple M4, 24 GiB RAM, current stable macOS 26 build, built-in microphone and speakers | Current Apple Silicon and current OS | NOT PROVISIONED |
| RH-W1 | ThinkPad X1 Carbon Gen 11, Intel Core i7-1355U, 16 GiB RAM, Windows 11 23H2 x64 | Minimum proposed Windows device class | NOT PROVISIONED |
| RH-W2 | Ryzen 7 PRO 7840U laptop, 16 GiB RAM, current stable Windows 11 x64 build | Second Windows CPU and driver family | NOT PROVISIONED |
| RH-C1 | Databricks Custom Model Serving Whisper large-v3-turbo on A10G in one supported US region, warm and `scale_to_zero_enabled=False` | Optional hosted Whisper | NOT PROVISIONED |

Record the exact model, OS build, patch level, architecture, driver versions, power source, battery mode, thermal state, application digest, runtime digest, model digest, VAD digest, thread count and execution provider for every run.
Repeat peripheral tests with the built-in devices, one wired USB headset, and one Bluetooth headset.
Repeat meeting tests with headphones and speakerphone.
Do not compare a result from one hardware row with a result from another row as if the environments were equal.

### 8.2 V-SPK-01: Tauri hotkey and window spike

1. Build one signed Tauri v2 harness with a stable application identity.
2. Implement the current `global-shortcut` candidate and native event timestamps.
3. Test `CommandOrControl+Shift+Space` and one administrator-approved alternative.
4. Record press, release, auto-repeat, foreground process, and target window.
5. Repeat each case 100 times after 20 warm-up cycles.
6. Test release of each key in a different order.
7. Test focus change, Escape, screen lock, sleep, session unlock, and application restart.
8. Suppress release delivery only in the harness and verify the 120-second safety stop.
9. If chord release fails, implement the native fallback for that OS.
10. On macOS, test `flagsChanged`, tap disable, and permission preflight for the fallback.
11. On Windows, use a dedicated message-loop thread for the fallback and enqueue work without blocking the callback.
12. Test a modifier-only binding only if product management requests it.
13. Create the mascot surface and response panel as separate windows.
14. Verify focus before and after mascot show, mascot click, response-panel show, and response-panel dismissal.
15. Test two monitors, all desktops or virtual desktops, full-screen applications, display scaling and reduced motion.
16. Fail the spike if any capture can start without a visible indicator or if a release remains unbounded.

**Exit evidence:** signed build digest, event traces, focus traces, videos, failure list, selected backend, and 100-cycle latency file.  
**Current result:** NOT RUN on macOS. NOT RUN on Windows.

### 8.3 V-SPK-02: audio, permission, and track spike

1. Use a fresh disposable OS user profile.
2. Verify microphone, keyboard, insertion, and system-audio permissions independently.
3. Deny each permission before capture.
4. Confirm no non-empty audio, transcript, or network request appears.
5. Grant only microphone permission and record a known sequence.
6. Grant process-audio permission and select one call process tree.
7. Play sequence A in the target process and sequence B in another process.
8. Verify that the process track contains A and excludes B.
9. Record microphone and process tracks with one monotonic timestamp source.
10. Verify 16 kHz mono STT copies and unchanged retained source tracks.
11. Repeat with headphones, speakerphone, USB headset and Bluetooth headset.
12. Revoke permission during capture when the test OS permits it.
13. Disconnect the active device and change the default device.
14. Close the target process and reopen it.
15. Verify the required stop, grace, and explicit-restart behavior.

**Exit evidence:** permission states, process-scope configuration, source tracks, timeline log, device-event log and network deny trace.  
**Current result:** NOT RUN on macOS. NOT RUN on Windows.

### 8.4 V-SPK-03: call-signal spike

1. Select Zoom and Microsoft Teams versions for the test matrix.
2. Record app version, OS build, process identifier, and permission state.
3. Start a meeting and observe available supported OS or app signals.
4. Mute and unmute the local call microphone.
5. Leave and rejoin the call.
6. Switch audio devices in the call.
7. Put the call on hold when supported.
8. Verify whether each observed signal means Motherbricks device ownership, call microphone state, or only process activity.
9. Return `Unknown` when no trustworthy signal exists.
10. Do not infer release from mute UI images, inaccessible windows, audio silence, or process CPU.
11. Verify a continuous trusted release stops after 30 seconds.
12. Verify reactivation inside 30 seconds cancels the stop.

**Exit evidence:** provider capability table by OS and call-app version, with primary API references where available.  
**Current result:** NOT RUN. Universal support remains unverified.

### 8.5 V-SPK-04: crash, sleep, disk, and recovery spike

1. Use test-only segment durations of 5, 10 and 300 seconds.
2. Kill the process at each journal state: open, page write, metadata flush, close, rename and transcript write.
3. Restart after each kill point.
4. Decode every closed segment and recover the open segment to the last valid page.
5. Compare recovered duration and sample sequence with the source generator.
6. Verify no automatic capture restart.
7. Fill a disposable volume through 2 GiB and 500 MiB free thresholds.
8. Verify warning, stop and preserved data.
9. Sleep the host during an open segment.
10. Wake it after one minute.
11. Verify `interrupted_by_sleep` and no automatic resume.
12. Run the complete four-hour recovery-time test after functional cases pass.

**Exit evidence:** kill-point matrix, decode reports, recovered sample diffs, disk trace, sleep trace and recovery timings.  
**Current result:** NOT RUN on macOS. NOT RUN on Windows.

### 8.6 V-SPK-05: insertion and clipboard spike

1. Test a native editor, browser text field, terminal, and target with accessibility support.
2. Save identifiable clipboard formats before insertion.
3. Delay cleanup and inject focus changes at controlled barriers.
4. Verify a changed target receives nothing.
5. Insert raw text once and return cleanup later.
6. Verify no second edit.
7. Change clipboard content after Motherbricks saves it.
8. Verify old content is not restored over the new content.
9. Block injection and verify Copy fallback.
10. On Windows, target an elevated editor from a non-elevated Motherbricks process.
11. Verify no elevation request and no insertion claim.
12. Run 100 focus races, 100 clipboard races and 30 complete dictation cycles.

**Exit evidence:** target and clipboard counters, race logs, application snapshots, Copy fallback videos and duplicate-insertion count.  
**Current result:** NOT RUN on macOS. NOT RUN on Windows.

### 8.7 V-BENCH-01: local STT and resource benchmark

1. Freeze the signed release build and all model digests.
2. Reboot the reference device.
3. Connect power for the primary run and record power mode.
4. Close unrelated user applications.
5. Record background processes and thermal state.
6. Load the model and run 20 warm-up clips.
7. Use 100 clips each in 2-, 10- and 30-second duration buckets.
8. Randomize clip order with a recorded seed.
9. Measure release-to-raw and chunk-close-to-raw with one monotonic clock.
10. Record RSS, CPU, thread count, execution provider and inference time.
11. Repeat with the documented 50 percent CPU background load.
12. Run 30 cold-load measurements separately.
13. Run the four-hour two-track meeting and drain the STT queue.
14. Measure audio gaps, drops, queue depth, RSS growth and recovery time.
15. Run a battery-power session as a secondary report.
16. Publish raw samples and percentile calculations with failures included.

**Current result:** NOT RUN. No local latency, WER, CPU, memory, battery or thermal result is available.

### 8.8 V-BENCH-02: hosted STT and cleanup benchmark

1. Use an approved non-production workspace with synthetic or consented audio.
2. Record workspace region, endpoint, model service, preview state, routing setting, service tier and date.
3. Keep the Whisper endpoint warm for interactive tests.
4. Measure cold scale-from-zero separately if an administrator enables it for a later batch use case.
5. Use the same audio clips and network conditions as the local run where practical.
6. Use a baseline network of at least 20 Mbit/s downstream, 5 Mbit/s upstream, 80 ms added round-trip latency and no intentional loss.
7. Add a constrained profile of 5 Mbit/s downstream, 1 Mbit/s upstream, 150 ms round-trip latency and 0.5 percent packet loss.
8. Run 20 warm-up and 100 measured requests per model and profile.
9. Measure request start, first response byte, complete raw transcript and cleanup completion.
10. Record timeout, HTTP status, provider error and retry count.
11. Do not count a fallback result as a successful provider-latency sample.
12. Run the cleanup corpus with tools disabled and bounded output.
13. Report verbatim fidelity separately from summary quality.
14. Do not infer diarization from source-track labels.
15. Compare standard and priority tiers only when both are supported for the selected model.
16. Do not claim that priority mode guarantees a latency target.

**Current result:** NOT RUN. No hosted STT or cleanup latency, availability, cost or fidelity result is available.

### 8.9 V-BENCH-03: accuracy and cleanup fidelity corpus

1. Freeze a public regression corpus and a separate consented product corpus.
2. Include at least 200 near-field dictation clips from at least 20 speakers.
3. Include at least 12 meeting recordings of 30 minutes each with varied rooms, microphones, accents, overlap, noise and speakerphone use.
4. Include at least 300 cleanup cases.
5. Include numbers, decimal values, currencies, dates, units, negation, names, Databricks terms, false starts, self-corrections, questions, commands and commitments.
6. Include malicious transcript instructions as data-only abuse cases.
7. Store participant consent and retention class with private corpus metadata.
8. Freeze reference transcripts and normalization rules before evaluation.
9. Report WER after the frozen normalization.
10. Report numeric-token recall before cleanup.
11. Report cleanup additions, deletions and protected-term changes.
12. Treat a changed fact, number, negation, person, commitment, or route as a critical semantic mutation.
13. Require zero critical mutations after validation in the release cleanup corpus.
14. Report each supported language separately before claiming that language.

**Current result:** CORPUS NOT PROVISIONED. NOT RUN.

### 8.10 Pending result register

| Result | Required evidence | Release disposition |
|---|---|---|
| Hotkey press and release | V-SPK-01 raw traces on all reference rows | BLOCKS MVP |
| Non-activating overlay and independent indicator | V-SPK-01 videos and focus logs | BLOCKS MVP |
| Permission denial and revocation | V-SPK-02 state and file evidence | BLOCKS MVP |
| Process-scoped two-track capture | V-SPK-02 tracks and scope manifests | BLOCKS dual-track MVP; mic-only release needs explicit director scope change |
| Trusted call microphone release | V-SPK-03 capability matrix | BLOCKS that signal per unsupported app; it does not block manual and silence policies |
| Safe insertion | V-SPK-05 race and elevated-target results | BLOCKS automatic insertion; Copy-only fallback can be considered by director |
| Local latency and resources | V-BENCH-01 complete raw samples | BLOCKS claimed performance and default-model approval |
| Local WER and numeric recall | V-BENCH-03 metrics | BLOCKS claimed accuracy and default-model approval |
| Hosted STT | V-BENCH-02 and workspace configuration | BLOCKS Cloud provider only |
| Cleanup latency and fidelity | V-BENCH-02 and V-BENCH-03 | BLOCKS cloud cleanup; raw fallback remains required |

## 9. Dependencies and interface consumers

| Interface | Team V contract | Consumer or reviewer |
|---|---|---|
| `HotkeyBackend` | Emits initial press, release, cancellation and backend-health events with monotonic timestamps. | Architecture, UX, Security |
| `CaptureLease` | Contains lease ID, MOG ID, session ID, source IDs, target process, owner state and acquisition time. | Architecture, UX, Security |
| `CaptureSession` | Exposes explicit start, normal stop, cancel, interrupt reason, source state and recovery manifest. | Architecture, UX |
| `CallSignalProvider` | Returns `Active`, `Released`, or `Unknown` with source and observation time. | Architecture, Security |
| `AudioJournal` | Stores separate source tracks, sequence numbers, monotonic times, segment state and recovery metadata. | Architecture, Memory, Security |
| `SttProvider` | Declares capabilities and input limits. Returns raw text, source timing, provider metadata and optional confidence. | Architecture, UX |
| `CleanupProvider` | Accepts raw text and protected terms. Returns versioned cleaned text or a typed failure. It cannot route. | Architecture, Security, UX |
| `InsertionService` | Revalidates target, inserts one selected version once, and returns inserted or copy-required. | Architecture, UX, Security |
| Privacy indicator | Native service owns its lifecycle. Capture cannot run without it. | UX, Security |
| Temporary data deletion | Journal and ASR chunks use the same local deletion operation as other MOG data. | Memory, Security |
| Performance evidence | Team V supplies raw local and cloud voice timings. Genie timing is outside these measurements. | Architecture |

## 10. Open issues

### Blocking verification items

1. **V-BLK-01:** Physical hotkey release, modifier ordering, focus loss and Escape behavior are pending on all reference devices.
2. **V-BLK-02:** Non-activating overlay and independent recording indicator behavior are pending on all reference devices.
3. **V-BLK-03:** Microphone and process-audio permission denial and revocation are pending.
4. **V-BLK-04:** Process-scoped capture and two-track alignment are pending for each supported OS and call app.
5. **V-BLK-05:** Insertion race, clipboard race and elevated-target behavior are pending.
6. **V-BLK-06:** Parakeet ONNX latency, memory, WER and numeric recall are pending on named hardware.
7. **V-BLK-07:** Calibrated confidence and timestamp availability in the selected converted runtime are pending.
8. **V-BLK-08:** Cleanup model availability, permission, latency and fidelity are pending in an approved workspace.
9. **V-BLK-09:** Hosted Whisper preview enablement, warm availability and latency are pending if Cloud Whisper is selected.
10. **V-BLK-10:** Gemini verbatim transcription, timestamp, diarization, region and routing behavior are pending if Cloud Gemini is selected.
11. **V-BLK-11:** A universal trusted call-microphone-release signal is not verified. Support must remain per provider and call app.
12. **V-BLK-12:** Four-hour audio continuity, memory growth and crash-recovery time are pending.

### Later-release issues

1. Linux and Wayland behavior is outside the proposed MVP support matrix.
2. Windows ARM64 needs a separate runtime and device matrix before support.
3. Speaker identity and cross-track diarization are not MVP capabilities.
4. Acoustic echo cancellation needs a separate speakerphone evaluation before it changes retained source audio.
5. Double-tap hands-free activation needs an accidental-start and accessibility evaluation.
6. Whole-system audio capture needs a separate privacy review and is not a fallback for process-scoped capture.
7. Mobile and web companions need reduced capture and insertion requirements rather than copied desktop claims.

### User-confirmation items

No existential user question blocks this requirements baseline.
The support matrix, latency gates, and quality gates can be reviewed by the director without user input.
A failure of the default local model would require a director decision between changing the runtime, changing the support floor, or changing the release scope.

## 11. Primary sources

All sources were accessed on 2026-11-14.

- **S-01:** Tauri, `@tauri-apps/plugin-global-shortcut`, including `ShortcutEvent.state` and `isRegistered` limitations. https://v2.tauri.app/reference/javascript/global-shortcut/
- **S-02:** Apple, `CGEventType`, including `keyDown`, `keyUp`, `flagsChanged`, `tapDisabledByTimeout` and `tapDisabledByUserInput`. https://developer.apple.com/documentation/coregraphics/cgeventtype
- **S-03:** Microsoft, `LowLevelKeyboardProc`, including key-up messages, message-loop requirement, callback timeout and silent hook removal. https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc
- **S-04:** Apple, “Capturing system audio with Core Audio taps,” including process selection, aggregate-device use, macOS 14.2 sample requirement and `NSAudioCaptureUsageDescription`. https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps
- **S-05:** Microsoft, “Application loopback audio capture,” including process-tree capture and build 20348 requirement. https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/
- **S-06:** Microsoft, `SendInput`, including UIPI and integrity-level restrictions. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
- **S-07:** Microsoft, `GetClipboardSequenceNumber`, including clipboard change tracking. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboardsequencenumber
- **S-08:** NVIDIA, `parakeet-tdt-0.6b-v3` model card, including input, languages, punctuation, capitalization, license and long-audio conditions. https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
- **S-09:** sherpa-onnx, NeMo transducer models, including `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`, artifact contents and microphone-plus-VAD examples. https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/nemo-transducer-models.html
- **S-10:** sherpa-onnx, Silero VAD, including supported v4 and v5 artifacts, sample rates and model sizes. https://k2-fsa.github.io/sherpa/onnx/vad/silero-vad.html
- **S-11:** Databricks, “Serve custom LLMs with Custom Model Serving,” updated 2026-11-11, including Public Preview status, Whisper large-v3-turbo, transcription task and scale-to-zero configuration. https://docs.databricks.com/aws/en/machine-learning/model-serving/serve-custom-llms
- **S-12:** Databricks, “Query audio and video models,” updated 2026-10-08, including inline audio, supported Gemini pay-per-token models and limitations. https://docs.databricks.com/aws/en/machine-learning/model-serving/query-audio-video-models
- **S-13:** Databricks, “Foundation model REST API reference,” updated 2026-11-11, including Chat Completions, temperature, token limits, streaming and structured output. https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/api-reference
- **S-14:** Databricks, `ai_transcribe`, including Beta status, SQL/notebook usage, segment timestamps, diarization and limits. https://docs.databricks.com/aws/en/sql/language-manual/functions/ai_transcribe
- **S-15:** Databricks, supported foundation models, including Meta Llama 3.1 8B Instruct model-service and endpoint names. https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/supported-models
- **S-16:** Databricks, priority pay-per-token, including supported models and the statement that priority does not guarantee a latency target. https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/priority-mode
- **S-17:** Apple, `NSPanel`, including floating-panel and key-window behavior. https://developer.apple.com/documentation/appkit/nspanel
- **S-18:** Microsoft, extended window styles, including `WS_EX_NOACTIVATE`, `WS_EX_LAYERED`, `WS_EX_TOPMOST` and `WS_EX_TRANSPARENT`. https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles
- **S-19:** Apple, `CGRequestListenEventAccess`, including macOS 10.15 availability. https://developer.apple.com/documentation/coregraphics/cgrequestlisteneventaccess()
- **S-20:** Microsoft, desktop-app privacy access management, including microphone allow/deny policy and fail-closed deny precedence. https://learn.microsoft.com/en-us/windows/client-management/manage-desktop-app-privacy-access
- **S-21:** Apple, `NSPasteboard.changeCount`, used to detect pasteboard changes. https://developer.apple.com/documentation/appkit/nspasteboard/changecount

