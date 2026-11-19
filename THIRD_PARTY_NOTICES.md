# Third-party notices

Bottle includes, ports, adapts or downloads the third-party components below. This file
lists the notices of requirements section 20.1 (SEC-21a, SEC-21b) and the notice for code
that Bottle adapts from Handy. The About screen of the app shows the same text. The test
`tests/third-party-notices.test.ts` checks that every notice is present.

## How to add an entry

Do these steps in the pull request that adds, ports or adapts third-party code:

1. Add one row to the summary table: `| <name> | <licence> | <how Bottle uses it> |`.
2. Add one section with the heading `## <name> (<licence>)`.
3. In the section, write the source URL and the commit or the version.
4. Copy the copyright line and the full licence text into a `text` code block. Do not change the text.
5. In each file that holds adapted Handy code, put this header at the top:
   `// Adapted from cjpais/Handy <path> @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.`
6. Add one row for each adapted file to the "Adapted files" table of the Handy section.
7. For code from another source, list the adapted files in the section of that source.
8. Run `pnpm test`. The test fails when a notice, a header or a row is missing.

| Component | Licence | How Bottle uses it |
|---|---|---|
| antigravity-bricks-plugin | MIT | Behaviour ported from `genie_bridge.py` and the skills |
| Handy (cjpais/Handy) | MIT | Code adapted for capture, hotkeys, speech recognition, paste, overlay and tray |
| NVIDIA Parakeet TDT 0.6B v3 (weights) | CC-BY-4.0 | Downloaded after install, SHA-256 pinned |
| sherpa-onnx (k2-fsa) | Apache-2.0 | Speech recognition runtime |
| onnxruntime | MIT | Model inference |
| Silero VAD v6 | MIT | Voice activity detection |
| Rive runtimes (`@rive-app/webgl2` / `@rive-app/canvas`, rive-wasm) | MIT | Mascot animation |
| Tauri, `tauri-nspanel`, `tauri-plugin-global-shortcut` | MIT / Apache-2.0 | Desktop shell |
| Gemini on Databricks | Applicable model terms | Model called through Databricks |
| Databricks CLI | Databricks License | Not bundled |
| shadcn/ui | MIT | Component source copied into `src/views/ui/` (ui-1, ADR 0106) |
| Radix UI (`@radix-ui/react-*`) | MIT | Accessible primitives behind `src/views/ui/` (ui-1, ADR 0106) |
| Motion (`motion`) | MIT | Animation in the hub, hub-only (ui-1, ADR 0106) |
| Tailwind CSS (`tailwindcss`, `@tailwindcss/vite`) | MIT | Hub-only utility CSS (ui-1, ADR 0106) |
| lucide-react | ISC | Icons in the hub (ui-1, ADR 0106) |
| clsx, tailwind-merge | MIT | The `cn()` class helper (ui-1, ADR 0106) |
| class-variance-authority | Apache-2.0 | Button variants (ui-1, ADR 0106) |
| objc2 Core Audio bindings, claxon | Apache-2.0 | System audio tap of call recordings; FLAC check in tests only |
| unsafe-libopus (libopus 1.3.1), ogg | BSD-3-Clause | Opus encoder and Ogg container of call recordings |

## antigravity-bricks-plugin (MIT)

Source: github.com/datasciencemonkey/antigravity-bricks-plugin, commit `a31fd2c`
(2026-10-24). Bottle ports behaviour from `genie_bridge.py` and the skills.
Ported files keep this header:

```text
MIT License

Copyright (c) 2026 Sathish Gangichetty

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### Adapted files

Add one row for each Bottle file that holds code or data from antigravity-bricks-plugin.

| Bottle file | antigravity-bricks-plugin path |
|---|---|
| `src/test-support/genie-tools.json` | `plugins/databricks-genie/scripts/genie_tools.json` (verbatim copy) |
| `src/test-support/genie-one-mcp.ts` (the contract loader and `contractProblems`) | `plugins/databricks-genie/scripts/genie_tools.json` |
| `crates/mog-core/src/token_broker.rs` (the token cache: refresh margin, `expires_in` / `expiry` reply fields, drop on 401) | `plugins/databricks-genie/scripts/genie_bridge.py` (`token`, `invalidate`) |
| `crates/mog-core/src/mcp_transport/client.rs` (R8: one retry on 401, one new session on a 404 with a session, Genie One off on a 404 without one) | `plugins/databricks-genie/scripts/genie_bridge.py` (`forward`, `call_upstream`, `start_session`) |
| `crates/mog-core/src/mcp_transport/tools.rs` (the tool allowlist and argument names) | `plugins/databricks-genie/scripts/genie_tools.json` |
| `crates/mog-core/src/mcp_transport/proxy.rs` (the agent's Genie One proxy: local `initialize` and `ping`, tool failures as tool results, only Bottle's own texts in an error) | `plugins/databricks-genie/scripts/genie_bridge.py` (`dispatch`, `tool_result`, `describe`) |
| `src/brain/components/genie-errors.ts` (the GL-5 status rules: 401 sign-in, 403 IP access list, 404 Genie One off, a non-JSON page; only Bottle's own texts show) | `plugins/databricks-genie/scripts/genie_bridge.py` (`forward`, `describe`) |
| `src/brain/components/poll-actor.ts` (after a poll times out, poll again with the same IDs and never ask again) | `plugins/databricks-genie/scripts/genie_bridge.py` (`late_text`, `OUT_OF_TIME`) |
| `src/brain/components/ask-events.ts` (an answer names its conversation, and the next call of that conversation stays in the workspace where it started: the G-22 clarify follow-up) | `plugins/databricks-genie/scripts/genie_bridge.py` (`conversation_in`, the `conversations` pin in `call_tool`) |
| `crates/mog-core/src/profile_service.rs` (T7: the config parse that keeps key names only and never a value it does not need, keys before the first header in DEFAULT, only `databricks-cli` profiles supported) | `plugins/databricks-genie/scripts/genie_bridge.py` (`config_sections`, `profiles`, `U2M`) |
| `crates/cli-runner/src/resolve.rs` (R5: after PATH, look in the usual install folders `/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin`, `~/bin`, because a GUI start gets a minimal PATH) | `plugins/databricks-genie/scripts/genie_bridge.py` (`cli_candidates`) |

## Handy (MIT)

Source: https://github.com/cjpais/Handy, commit `aca8985`. Bottle adapts code from Handy
for the capture, hotkey, speech recognition, paste, overlay and tray features. Each file
that holds adapted Handy code has this header, with `<path>` set to the Handy source path:

```
// Adapted from cjpais/Handy <path> @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
```

```text
MIT License

Copyright (c) 2025 CJ Pais

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### Adapted files

Add one row for each Bottle file that holds adapted Handy code.

| Bottle file | Handy path |
|---|---|
| `crates/mog-core/src/audio_device/cpal_backend.rs` | `src-tauri/src/managers/audio.rs` |
| `crates/mog-core/src/audio_device/sink.rs` | `src-tauri/src/audio_toolkit/audio/sink.rs` |
| `crates/mog-core/src/hotkey_macos.rs` | `src-tauri/src/secure_input.rs` |
| `crates/mog-core/src/insertion.rs` | `src-tauri/src/clipboard.rs` |
| `crates/mog-core/src/insertion/macos.rs` | `src-tauri/src/input.rs` |
| `crates/mog-core/src/insertion/windows.rs` | `src-tauri/src/utils.rs` |
| `crates/mog-core/src/stt_model.rs` | `src-tauri/src/managers/model/download.rs` |
| `spikes/sp-03/src/clamshell.rs` | `src-tauri/src/helpers/clamshell.rs` |
| `spikes/sp-03/src/secure_input.rs` | `src-tauri/src/secure_input.rs` |
| `src-tauri/src/macos_panel.rs` | `src-tauri/src/overlay.rs` |
| `src/brain/components/cleanup-prepass.ts` | `src-tauri/src/audio_toolkit/text.rs` |

## NVIDIA Parakeet TDT 0.6B v3 (weights, CC-BY-4.0)

Speech recognition uses NVIDIA Parakeet TDT 0.6B v3 (https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3), © NVIDIA, licensed under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/), converted to ONNX and quantised to int8 by the k2-fsa/sherpa-onnx project. Modifications: ONNX export and int8 quantisation.

Bottle downloads the weights only from Hugging Face or the sherpa-onnx releases,
and checks each file against a pinned SHA-256 (SEC-20b). Bottle adds no
restrictions to the CC-BY-4.0 licence.

## sherpa-onnx (k2-fsa, Apache-2.0)

Source: https://github.com/k2-fsa/sherpa-onnx. The licence text of its LICENSE
file follows. The upstream repository has no NOTICE file at the time of
writing; if a release that Bottle uses adds one, its text must be added here.
Bottle links the sherpa-onnx 2.0.1 static libraries (with onnxruntime) only in the
`local-stt` build of `mog-core` (#45, ADR 0049).

```text
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```

## onnxruntime (MIT)

Source: https://github.com/microsoft/onnxruntime.

```text
MIT License

Copyright (c) Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Silero VAD v6 (MIT)

Source: https://github.com/snakers4/silero-vad.

```text
MIT License

Copyright (c) 2020-present Silero Team

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Rive runtimes (MIT)

`@rive-app/webgl2` / `@rive-app/canvas`, built from rive-wasm
(https://github.com/rive-app/rive-wasm).

Bottle ships `@rive-app/canvas` 2.50.0 (npm, exact pin). SP-01 chose this runtime
provisionally (#10, ADR 0012).

```text
MIT License

Copyright (c) 2020-2021 Rive

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Tauri (MIT / Apache-2.0)

Source: https://github.com/tauri-apps/tauri. Tauri is a project of The Tauri
Programme in the Commons Conservancy. It is dual-licensed under MIT or
Apache-2.0 (Apache-2.0 text: see sherpa-onnx above). MIT text:

```text
MIT License

Copyright (c) 2017 - Present Tauri Apps Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## tauri-plugin-global-shortcut (MIT / Apache-2.0)

Source: https://github.com/tauri-apps/plugins-workspace (plugins/global-shortcut).
Dual-licensed under MIT or Apache-2.0 (Apache-2.0 text: see sherpa-onnx above).
MIT text:

```text
MIT License

Copyright (c) 2017 - Present Tauri Apps Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## tauri-nspanel (MIT / Apache-2.0)

Source: https://github.com/ahkohd/tauri-nspanel. Dual-licensed under MIT or
Apache-2.0 (Apache-2.0 text: see sherpa-onnx above). MIT text:

```text
MIT License

Copyright (c) 2023 - Present Victor Aremu

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Gemini on Databricks (applicable model terms)

Bottle can call Gemini models served by Databricks. Bottle ships no Gemini code
or weights. The customer accepts the Gemini terms (ADM-04) before the models are
used.

## objc2 Core Audio bindings, claxon (Apache-2.0)

Call recording (ADR 0103) uses these crates. Bottle copies no code from them.

- `objc2-core-audio`, `objc2-core-audio-types`, `objc2-core-foundation` and `objc2-avf-audio`
  0.3.2 (https://github.com/madsmtm/objc2): the Rust bindings to the macOS Core Audio process
  tap and to `AVAudioApplication` (the Microphone prompt). They were already in Cargo.lock
  through cpal; ADR 0103 makes them direct macOS dependencies of `mog-core`. They are
  licensed "Zlib OR Apache-2.0 OR MIT"; Bottle uses them under Apache-2.0 (text: see
  sherpa-onnx above). Copyright the objc2 contributors.
- `claxon` 0.4.3 (https://github.com/ruuda/claxon): a FLAC decoder, a dev-dependency only. The
  tests decode Bottle's FLAC parts with it. It is not in the app. Apache-2.0 (text: see
  sherpa-onnx above). Copyright 2014 Ruud van Asseldonk.

## unsafe-libopus (libopus 1.3.1), ogg (BSD-3-Clause)

Call recording (ADR 0103) encodes each recording as one Ogg Opus file (16 kHz mono, 16 kbps).
Bottle copies no code from these crates; it links them as pinned Cargo dependencies.

- `unsafe-libopus` `=0.2.0` (https://github.com/DCNick3/unsafe-libopus): libopus 1.3.1
  (https://opus-codec.org) translated from C to Rust with c2rust. It needs no C toolchain.
  BSD-3-Clause, the libopus licence:

```text
Copyright 2001-2011 Xiph.Org, Skype Limited, Octasic,
                    Jean-Marc Valin, Timothy B. Terriberry,
                    CSIRO, Gregory Maxwell, Mark Borgerding,
                    Erik de Castro Lopo

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of Internet Society, IETF or IETF Trust, nor the
names of specific contributors, may be used to endorse or promote
products derived from this software without specific prior written
permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

Opus is subject to the royalty-free patent licenses which are
specified at:

Xiph.Org Foundation:
https://datatracker.ietf.org/ipr/1524/

Microsoft Corporation:
https://datatracker.ietf.org/ipr/1914/

Broadcom Corporation:
https://datatracker.ietf.org/ipr/1526/
```

- `ogg` `=0.9.2` (https://github.com/RustAudio/ogg): the Ogg container. BSD-3-Clause:

```text
Copyright (c) 2016-2017 est31 <MTest31@outlook.com> and contributors
Copyright (c) 2002-2015 Xiph.org Foundation

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of the Xiph.org Foundation nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE FOUNDATION
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Databricks CLI (Databricks License)

The Databricks CLI is not bundled with Bottle; it is deployed by the customer.
With consent, Bottle downloads the official release from
github.com/databricks/cli/releases and checks it against the release SHA-256
checksum file before it unpacks it (W-17d, SEC-20b).

## shadcn/ui (MIT)

Source: https://github.com/shadcn-ui/ui. Bottle copies shadcn/ui component source by hand into
`src/views/ui/` (ui-1, ADR 0106; the shadcn CLI is not a build or runtime dependency), adapted to
this repo's strict TypeScript config. Each copied file names its shadcn source doc page in its
own header comment. shadcn/ui ships no npm package and no version number of its own (it is
copied source, not a dependency); each file's header comment instead names the doc page it was
copied from, which is the closest thing to a version this project has.

```text
MIT License

Copyright (c) 2023 shadcn

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### Adapted files

`src/views/ui/button.tsx`, `tooltip.tsx`, `checkbox.tsx`, `switch.tsx`, `radio-group.tsx`,
`collapsible.tsx`, `input.tsx`, `utils.ts` (`cn()`) — each copied from the matching page of
ui.shadcn.com/docs/components/ and adapted for `verbatimModuleSyntax`/`exactOptionalPropertyTypes`.
`src/views/hub/HubSidebar.tsx` and `use-hub-sidebar.ts` are hand-written (shadcn's Sidebar
*pattern*, not its copy-in: the hidden-by-default/pin behaviour is this repo's own spec, ADR 0106
decision 6). `dialog.tsx`, `scroll-area.tsx`, `separator.tsx`, `tabs.tsx` and `sonner.tsx` were
copied in during the first pass but nothing imports them (review of #602): removed, along with
their Radix/`sonner` dependencies below, rather than kept unused. `src/views/WipeDialog.tsx` uses
`@radix-ui/react-dialog` directly (not through a `src/views/ui/` copy-in), so that one dependency
stays.

## Radix UI (MIT)

Source: https://github.com/radix-ui/primitives. Bottle depends on these npm packages (not copied
in; versions pinned in `package.json`): `@radix-ui/react-checkbox` 1.4.1, `-collapsible` 1.2.1,
`-radio-group` 1.5.1, `-slot` 1.5.0, `-switch` 1.4.1 and `-tooltip` 1.4.1 sit behind the
shadcn/ui components in `src/views/ui/`; `-dialog` 1.3.1 backs `src/views/WipeDialog.tsx`
directly (its focus trap and Escape-to-close), with no `src/views/ui/dialog.tsx` copy-in.

```text
MIT License

Copyright (c) 2022 WorkOS

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Motion (MIT)

Source: https://github.com/motiondivision/motion (motion.dev). npm package `motion`, version
14.13.0, imported as `motion/react`. Used for the hub sidebar's width/slide, the active-item
highlight, and the hand-built feedback effects in `src/views/ui/motion-effects.tsx` (ui-1, ADR
0106).

```text
MIT License

Copyright (c) 2024 Motion (https://motion.dev) B.V.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Tailwind CSS (MIT)

Source: https://github.com/tailwindlabs/tailwindcss. npm packages `tailwindcss` and
`@tailwindcss/vite`, both version 4.3.5. Hub-only (ui-1, ADR 0106): `src/views/hub/tailwind.css`;
`mog.html` and `card.html` never load it.

```text
MIT License

Copyright (c) Tailwind Labs, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## lucide-react (ISC)

Source: https://github.com/lucide-icons/lucide. npm package `lucide-react`, version 1.72.0;
icons render as inline SVG, bundled (no CDN sprite).

```text
ISC License

Copyright (c) 2026 Lucide Icons and Contributors

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH
REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY
AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM
LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR
OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR
PERFORMANCE OF THIS SOFTWARE.
```

Some Lucide icons (used by this project: `chevron-down`, `chevron-up`, among others) are
derived from the Feather project and carry their own MIT sub-licence, per `lucide-react`'s
`LICENSE` file:

```text
The MIT License (MIT) (for icons derived from the Feather project)

Copyright (c) 2013-present Cole Bemis

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## clsx, tailwind-merge (MIT)

Sources: https://github.com/lukeed/clsx and https://github.com/dcastil/tailwind-merge. npm
packages `clsx` 2.1.1 and `tailwind-merge` 3.7.0, combined as the `cn()` helper in
`src/views/ui/utils.ts`.

```text
MIT License

Copyright (c) Luke Edwards <luke.edwards05@gmail.com> (lukeed.com)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

```text
MIT License

Copyright (c) 2021 Dany Castillo

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## class-variance-authority (Apache-2.0)

Source: https://github.com/joe-bell/cva. npm package `class-variance-authority`, version 0.7.1,
used for the Button variants in `src/views/ui/button.tsx`. Apache License 2.0; the full licence
text is already reproduced above in the sherpa-onnx section of this file.
