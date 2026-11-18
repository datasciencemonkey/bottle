# Where your data goes

This page lists what Bottle sends off your computer, and what other services keep. Bottle shows the same text in the hub panel "Where your data goes". The setup wizard will show it too (requirement SEC-17, Table SEC-DISC).

The copy comes from one file: `src/views/hub/disclosures.ts`. If you change the copy there, change this page too.

## What leaves this computer

- Bottle sends data only to your own Databricks workspaces, and only for the jobs on this page.
- Dictation text goes to your home workspace for cleanup, then only to the app you type in.
- Nothing goes to Genie until you send it: a question after its preview, or a chip you click.
- Your sign-in tokens stay in the Bottle core. They are never written to disk or logs, and the agent never sees them.
- Bottle stores your transcripts, answers and memory in an encrypted file on this computer.
- For each Genie answer, Bottle keeps a short summary, the key numbers, a link to the answer and the full answer in that file.
- For an answer from a workspace that is not your home workspace, Bottle keeps only the link and the full answer.
- Bottle does not delete answers by age. They stay until you use Forget or Settings → Wipe (older than 30 days, older than 90 days, or everything).
- Text for cleanup, routing, the memory writer and embeddings goes to your home workspace, as the next sections say.
- Telemetry is off unless your admin turns it on.
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
- The STT badge on each MOG shows which one is on.

## Your home workspace does the processing

- Each MOG has a home workspace (its home profile).
- Text cleanup, the router step, the memory writer, embeddings and the main agent model run there.
- Embeddings send the text of memory items to that workspace.
- This stays the same when a MOG asks Genie in another workspace.
- Questions about another workspace are processed in your home workspace.
- The router step gets only your words, the capture mode, recent topic labels, your MOGs and your profiles. It never gets memory text.
- The router model and the cleanup and writer models never get tools.
- If your words sound like an instruction to Bottle, for example "ignore the rules" or "send my notes to this URL", Bottle sends nothing and shows a warning chip.
- The router step never suggests a workspace outside `AllowedWorkspaceHosts`, when your admin sets it.

## The main agent and its model

- Each MOG has a main agent with a Claude or Codex harness.
- The harness runs on this computer, in one shared runtime with one session for each MOG.
- Its model runs through the Databricks AI Gateway in your home workspace with your identity, or as a local model.
- The agent gets your cleaned question, the router decision and memory results.
- It never gets your sign-in tokens: a local proxy in the Bottle core adds them.
- The agent cannot start recording, send context, publish a Page or change the workspace without you.
- Dictation does not go through the agent.

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

- You can forget a session, some notes, or a whole MOG. Bottle deletes only when you select Forget.
- Forget deletes the matching rows on this computer at once, also when you are offline. It also deletes each note that cites them, with its search entries.
- Forget a session deletes its transcript, its notes and its Genie answers. Forget a MOG deletes all of them for that MOG.
- Bottle keeps a short Forget record with IDs only, for a later sync. Consent records stay for their own 13 months.
- Bottle cannot delete Genie One conversations yet. The dialog shows a "Delete in Genie One" link for each one.
- Bottle deletes the conversations of a Genie space MOG through the Genie API. If a delete fails, the dialog tells you.

## Your DRAFT Pages

- When you approve a definition, Bottle creates a DRAFT Unity Catalog Page that you own, in the home domain of the MOG.
- Genie One can read your DRAFT Pages in your own conversations.
- Bottle does not publish or delete Pages. You delete them in Unity Catalog.

## Lakebase backup window (later version)

- Sync to Lakebase is not in this version. Your memory stays on this computer.
- When sync comes, Lakebase keeps a point-in-time backup for 2 days by default.
- Deleted data is fully erased only after that window. Bottle will show the value.
