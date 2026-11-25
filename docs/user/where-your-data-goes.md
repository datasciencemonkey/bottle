# Where your data goes

This page lists what Bottle sends off your computer, and what other services keep. Bottle shows the same text in the hub panel "Where your data goes" (select the heading to open it). The setup wizard will show it too (requirement SEC-17, Table SEC-DISC).

The copy comes from one file: `src/views/hub/disclosures.ts`. If you change the copy there, change this page too.

## What leaves this computer

- Bottle sends data only to your own Databricks workspaces, and only for the jobs on this page.
- Dictation text goes to your home workspace for cleanup, then only to the app you type in.
- Nothing goes to Genie until you send it: ⌥G during the talk-key hold, or an Ask Genie button you press.
- Your sign-in tokens stay in the Bottle core. They are never written to disk or logs, and the agent never sees them.
- Bottle stores your transcripts, answers and memory in an encrypted file on this computer.
- For each Genie answer, Bottle keeps a short summary, the key numbers, a link to the answer and the full answer in that file.
- For an answer from a workspace that is not your home workspace, Bottle keeps only the link and the full answer.
- Bottle does not delete answers by age. They stay until you use Forget or Settings → Wipe (older than 30 days, older than 90 days, or everything).
- Text for cleanup, routing, the memory writer and embeddings goes to your home workspace, as the next sections say.
- Telemetry is off unless your admin turns it on. Telemetry holds only fixed names, times and counts, never your words.
- Logs hold no transcript, question or answer, unless you and your admin both turn on content logging.
- Databricks logs each model call in system.ai_gateway.usage: who called, from where and how many tokens, but no content.
- Databricks keeps model requests in an abuse-check store for up to 30 days.
- A model on the Gemini global endpoint can send requests to other regions.
- If an admin turns on inference tables, Databricks stores model requests there. They stay off for speech-to-text.

## Where speech-to-text runs

- Speech-to-text runs on this computer by default, with the local Parakeet model.
- Push-to-talk and hands-free audio stay in memory, and Bottle never writes them to a file. Bottle does not record before you press the talk key or start hands-free. After transcription, Bottle overwrites the audio in memory with zeros.
- Your computer can move memory to its swap or page file on disk. On Windows the page file is not encrypted by default. macOS encrypts swap by default. Bottle tracks page locks for this audio in #500.
- Meeting audio is kept encrypted on disk and deleted within 5 minutes after transcription, unless your admin keeps it longer.
- Bottle uses Databricks speech-to-text only if you or your admin turn it on, after a check passes.
- Then audio goes to your home workspace in short chunks, and Bottle does not store it there.
- If you set Dictation engine to Databricks, the audio of each talk-key hold goes to your home workspace to be transcribed.
- Bottle sends it as one audio file, and does not store it there. If that call fails, Bottle uses the local model.
- The STT badge on each MOG shows which one is on.
- If your admin sets local-only speech-to-text, no audio leaves this computer, also when local speech-to-text fails.
- The local model knows 25 European languages. For another language, Bottle uses Databricks speech-to-text only if it is on. If it is off, the MOG shows "Transcription of this language is not available on this device."
- If no speech-to-text gives a result, the MOG shows "Could not transcribe. Retry". The Retry button comes in a later version; until then, press the talk key again.

## The local speech model

- The setup step "Local speech" asks before Bottle downloads the speech model (487 MB). Nothing downloads before you select "Download".
- If you select "Not now", local speech stays off. You can download the model later from the same step.
- Bottle keeps your answer in the model folder, so it does not ask again at each start.
- The model comes only from the sherpa-onnx release on GitHub or from its Hugging Face mirror. You select the source.
- On macOS the model folder is `~/Library/Application Support/Bottle/models`. On Windows it is `%LOCALAPPDATA%\Bottle\models`. Bottle needs no admin rights for it.
- You can cancel the download. Select Retry to continue from where it stopped.
- Bottle checks each file against a fixed checksum. If a file does not match, Bottle deletes it and the install stops.
- The download does not use a proxy server yet (#535).

## Your home workspace does the processing

- Each MOG has a home workspace (its home profile).
- Text cleanup, the router step, the memory writer, embeddings and the orchestrator model run there.
- Embeddings send the text of memory items to that workspace.
- If a memory item sounds like an instruction to the assistant, for example a Genie answer or a meeting line that says "ignore previous instructions", Bottle keeps the item on your device but marks it quarantined. A quarantined item is never embedded, never added to a question to Genie and never given to the orchestrator model.
- "Remember this" pins an item only when you say it or click it. The orchestrator model cannot pin an item.
- This stays the same when a MOG asks Genie in another workspace.
- Questions about another workspace are processed in your home workspace.
- The router step gets only your words, the capture mode, recent topic labels, your MOGs and your profiles. It never gets memory text.
- The router model and the cleanup and writer models never get tools.
- If your words sound like an instruction to Bottle, for example "ignore the rules" or "send my notes to this URL", Bottle sends nothing and shows a warning chip.
- The router step never suggests a workspace outside `AllowedWorkspaceHosts`, when your admin sets it.

## The orchestrator model

- Bottle uses one orchestrator model. It runs through the Databricks AI Gateway in your home workspace, with your identity.
- You pick the model in Settings, Workspace. The list shows the system.ai chat models.
- It never gets your sign-in tokens: the Bottle core adds them.
- Each model call sets store to false.
- For dictation, the model cleans up the text. It has no tools.
- Bottle pastes the model text only when its words match your words. Otherwise Bottle pastes the local text.
- The cleanup step waits 1.5 seconds at most. Then Bottle pastes the local text.
- The setting Clean up dictation switches this step off. Then dictation text stays on this computer.
- When you press ⌥G, Bottle sends your question to the orchestrator model in your home workspace.
- Bottle asks your home workspace first. It asks your other workspaces only when home has no answer or your question names one. It asks only workspaces you turned on.
- The model has two tools: ask Genie in one workspace, and submit the answer.
- Every ask goes through a guard in the Bottle core. The guard sets a budget and blocks quarantined text.
- The model cannot start recording, send context, publish a Page or change the workspace.
- The model sees the answers of every workspace it asks, and it can combine them into one answer.
- A combined answer cites each workspace with a numbered marker. Each marker links to the answer of that workspace.
- If the model is not reachable, Bottle asks Genie One in your home workspace and shows a short notice.
- Bottle copies each finished answer to the clipboard on this Mac only. The setting Copy Genie answers to the clipboard switches this off.

## Genie history

- Genie keeps the questions that Bottle sends in your Genie history in that workspace.
- Your admins can also see them in system.access.audit and system.query.history.
- You cannot delete these system table rows.
- Wipe in Settings can also delete the Genie conversations that Bottle created, where Genie allows it.

## Genie One memory

- Genie One can keep its own memory of your conversations on the Databricks server.
- Bottle does not control that memory.
- Forget in Bottle gives you a link to review your Genie One memories.

## Forget

- In the hub, select "More" next to a MOG, then "Forget memory". Bottle deletes only when you select Forget in the dialog.
- Bottle can also forget one session or some notes.
- Forget deletes the matching rows on this computer at once, also when you are offline. It also deletes each note that cites them, with its search entries.
- Forget a session deletes its transcript, its notes and its Genie answers. Forget a MOG deletes all of them for that MOG.
- Bottle keeps a short Forget record with IDs only, for a later sync. Consent records stay for their own 13 months.
- Bottle cannot delete Genie One conversations yet. The dialog shows a "Delete in Genie One" link for each one.
- Bottle deletes the conversations of a Genie space MOG through the Genie API. If a delete fails, the dialog tells you.

## Wipe

- Bottle does not delete your transcripts, dictation history, Genie answers or notes on its own.
- In the hub, open Wipe. Select one option: older than 30 days, older than 90 days, or everything. Then select Wipe and confirm.
- Wipe deletes the matching data of every MOG on this computer at once, also when you are offline. Search no longer finds it.
- A Wipe by age deletes only the data older than the option. A newer note stays, also when it cites an older answer.
- "Everything" leaves no transcript, dictation, answer or note. Your MOGs and their settings stay.
- Select "Also delete Genie conversations" to delete the Genie conversations of the deleted answers too. Bottle cannot delete Genie One conversations yet, so the dialog shows a "Delete in Genie One" link for each one.
- Some records hold no content and expire on their own: the consent records after 13 months and the agent action log after 30 days. A Wipe does not delete them.
- The Wipe panel shows how long Bottle keeps each kind of data. A lock and "Set by your admin" show a value that your admin sets. You cannot change it.
- The Wipe button does not work yet in this build. The dialog says that nothing changed (issue #525).

## Your DRAFT Pages

- When you approve a definition, Bottle creates a DRAFT Unity Catalog Page that you own, in the home domain of the MOG.
- Genie One can read your DRAFT Pages in your own conversations.
- Bottle does not publish or delete Pages. You delete them in Unity Catalog.

## Lakebase backup window (later version)

- Sync to Lakebase is not in this version. Your memory stays on this computer.
- When sync comes, Lakebase keeps a point-in-time backup for 2 days by default.
- Deleted data is fully erased only after that window. Bottle will show the value.
