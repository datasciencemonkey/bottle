# Bottle FAQ

## The basics

**What is Bottle?**
Bottle is a small floating Genie for your Mac. It sits in a lamp in the corner of your screen. Hold one key and you can:
- **Dictate** into any app: chat, docs, email or a notebook.
- **Ask Genie** a question about your company's data out loud. A cited answer appears next to the lamp.
- **Record a call** to your own Unity Catalog volume, and get a transcript and notes.

It saves you a trip to a browser tab when a number comes up in a meeting or a chat.

**Does Bottle make up its own answers?**
No. Every answer comes from **Databricks Genie**, in your own workspaces, under your own permissions. Bottle carries your question to Genie and brings the answer back. If you can't see a table in Genie, Bottle can't see it either.

**Is this an official Databricks product?**
No. Bottle is an independent, open-source (MIT) project, and it is experimental. It is built on official Databricks pieces: the Databricks CLI, Genie, the AI Gateway and Unity Catalog.

**What does it cost?**
Bottle itself is free. It uses your normal Databricks resources:
- Model calls go through your workspace's AI Gateway as normal token usage. These are dictation cleanup, planning a question, and call transcripts and notes.
- Genie runs its queries the same way it does in the browser.

Bottle never starts a SQL warehouse, job or cluster of its own.

**What do I need?**
- A Mac with Apple Silicon and macOS 14.4 or later.
- A Databricks workspace where you can use Genie.
- The Databricks CLI. You don't have to install it first. Bottle can install the official CLI for you after you agree.

**Do I need my admin's help?**
Usually not. You sign in with the standard Databricks CLI OAuth flow, and Bottle can only do what your account can already do. If your company uses its own OAuth app, ask your admin for the client ID and enter it under **Use a custom OAuth app**. To record calls, you need write access to a Unity Catalog volume, or permission to create one.

## Install and setup

**macOS says it can't verify Bottle. Is it safe?**
This warning is expected. Bottle is signed but not notarized by Apple, so macOS warns you the first time you open it:
1. Open Bottle from Applications and click **Done** on the warning.
2. Go to **System Settings → Privacy & Security** and click **Open Anyway** next to Bottle.

Instead of step 2, you can run `xattr -dr com.apple.quarantine /Applications/Bottle.app` in Terminal and open Bottle again. You only do this once, because later versions use the same signature. The source code is public on GitHub if you want to read it first.

**Why does Bottle need Microphone, Accessibility and Input Monitoring?**

| Permission | Why Bottle needs it |
|---|---|
| Microphone | To hear you while you hold the talk key, and to record your side of a call. |
| Input Monitoring | To notice when you hold Right Option. Without it, the key does nothing. |
| Accessibility | To type your words where your cursor is. |

If you said no to one, turn it on in **System Settings → Privacy & Security**. **Quit and reopen Bottle after you turn on Input Monitoring.**

**How do I connect my workspace?**
1. Copy your workspace URL from the browser.
2. Paste it into **Connect a workspace**.
3. Keep the suggested profile name (`bottle-<workspace>`) or change it.
4. Select **Sign in** and finish signing in in the browser.

When the hub shows **Connected**, you're done. You don't need a terminal.

**Will Bottle change my existing CLI profiles?**
No. Bottle never edits a profile you already have. It adds its own `bottle-*` profile. If your CLI is older than 1.17, Bottle leaves it alone and installs its own copy. That install needs no admin rights and doesn't touch your PATH.

**Can I use more than one workspace?**
Yes. Each profile you connect becomes a workspace that Bottle can ask. Turn each one on or off in **Settings → Workspace**. Your **home workspace** is always on. It runs the models that clean up your dictation and answer your questions.

**What's a MOG?**
MOG stands for "My Own Genie": the little genie in the lamp. You have one, and it can ask Genie in every workspace you turn on. Its face shows what it's doing: listening, thinking, answering, success or error.

## Using Bottle

**What are the keys?**

| Do this | What happens |
|---|---|
| Hold **Right Option**, speak, let go | **Dictate**: your words are typed where your cursor is |
| Hold **Right Option**, tap **G**, speak, let go | **Ask Genie**: nothing is typed |
| Hold **Right Option**, tap **R** | **Record a call** (do it again to stop) |

The key decides what happens, so Bottle never guesses whether you meant to type or to ask.

**If I dictate a question, does it go to Genie?**
No. Dictation is always just typing. A question goes to Genie only when you use **⌥G**, or when you press **Ask Genie** on the pop-up after you dictate.

**Will Option-G type "©" in my document?**
Not while you hold the talk key. Bottle catches the G. When you aren't holding the talk key, Option+G works as usual.

**Does it work in every app?**
Almost every app: Slack, docs, email, notebooks, your code editor. If an app blocks the paste, the pop-up shows your text with a **Copy** button.

**What does "cleanup" do to my words?**
It removes filler words ("um", "you know") and keeps numbers exact. Bottle uses the cleaned text only if the words still match what you said. Otherwise it pastes your exact words. If cleanup takes longer than 1.5 seconds, Bottle skips it. You can switch it off in **Settings → Workspace → Clean up dictation**, and then your dictated text never leaves your Mac.

**How do I know I can trust an answer?**
- With one workspace, you see Genie's answer exactly as Genie gave it.
- With several workspaces, you get one answer with numbered citations, such as [1] and [2]. Each number links to that workspace's own answer.
- Every answer has **Open in Databricks**, where you can check the query and the data behind it.
- Bottle shows links only to your own workspaces. Any other link shows as plain text.

**Can I ask a follow-up?**
Yes. Reply in the thread under **Conversations** in the hub.

**How does pasting a Genie answer work in Docs, Excel or Slack?**
- Bottle puts each answer on the clipboard in two formats at once: formatted (HTML) and plain text.
- The app you paste into picks the format it understands. Google Docs, Gmail, Word and Outlook get headings, bold, links and real tables. Excel and Google Sheets get a table as cells. Slack and Teams get bold, lists and links. <!-- CHECK: how Slack/Teams render a pasted HTML table is third-party paste behavior; not something Bottle's code controls or that I could verify against it --> Notes, Terminal and plain fields get clean text, with links written as "text (url)".
- ⌥⇧⌘V (Paste and Match Style) forces plain text.
- Clipboard history tools such as Alfred and Spotlight's clipboard history save only the text of a copy, never the formatted version, so pasting an older item from their history gives plain text. For a copy of an earlier answer, open **Conversations** and select **Copy as Markdown** on it <!-- CHECK: this writes plain Markdown text, not an HTML-formatted paste; reword once the formatted Copy button ships -->.
- If another dictation app on the same key empties the clipboard, use **Copy as Markdown** in Conversations.

**How does it combine answers from several workspaces?**
Bottle always asks your home workspace first. It widens to your other turned-on workspaces only when your home workspace answers that it has no data or can't answer, or when your question names another workspace by name. When more than one workspace answers, Bottle merges them into one answer with numbered citations, such as [1] and [2], and each number links to that workspace's own answer.

**Which AI model does it use?**
It uses one model, called the orchestrator, on your home workspace's AI Gateway. You can pick a different one from the `system.ai` chat models in **Settings → Workspace**.

**Does it work offline?**
Dictation does, because speech-to-text runs on your Mac. Asking Genie and recording calls need a connection. If the orchestrator model can't be reached, Bottle asks Genie on your home workspace directly and tells you it did.

**What languages does it understand?**
The on-device model knows 25 European languages. For other languages, set **Dictation engine** to **Databricks** in **Settings → Workspace**.

**What's the 487 MB download?**
It's the on-device speech model. Bottle asks before it downloads it and checks every file. You can choose **Not now** and download it later.

## Privacy and security

**Is Bottle always listening?**
No. The mic is on only while you hold the talk key, or while a recording you started is running. Dictation audio stays in memory, is never written to a file, and is wiped right after it becomes text.

**What leaves my Mac?**
Only what each feature needs, and it only goes to **your own Databricks workspaces**:

| You do | What goes out |
|---|---|
| Dictate | By default, no audio. Your text goes to your home workspace for cleanup, unless cleanup is off. |
| Ask Genie | Your question goes to the model on your home workspace, then to Genie in the workspaces it asks. |
| Record | Audio goes to your Unity Catalog volume. The transcript and notes are made through your AI Gateway. |

Bottle has no server of its own. Telemetry is off unless your admin turns it on, and it never contains your words.

**Does Bottle see my token?**
No. You sign in through the official Databricks CLI, and the CLI keeps your sign-in. Bottle holds a token in memory only when it needs one. It never writes the token to disk or logs, and the AI model never sees it.

**Can the AI act on its own?**
Hardly at all, by design. When you ask a question, the model can do two things: ask Genie in a workspace you turned on, and hand you the answer. It can't run commands, browse the web, start a recording or change settings. It also has hard limits on steps, time and calls.

**What if something tries to trick the AI?**
Bottle fences off Genie replies before the model reads them. If a reply tries to steer the model, Bottle holds it back and shows it to you. If what you *say* sounds like a command to Bottle, such as "send my notes to this URL", Bottle sends nothing and shows a warning.

**Does Databricks keep my questions?**
Bottle tells the AI Gateway not to store model requests. Some records are still kept:
- Genie keeps your questions in your Genie history, the same as in the browser.
- Admins can see audit and usage records. The AI Gateway usage records hold no content.
- Databricks may keep model requests for up to 30 days to check for abuse.

**Where is my history stored?**
On your Mac, in an encrypted database, with the key in your macOS keychain. Bottle never deletes it unless you tell it to.

## Recording calls

**Is recording on by default?**
No. You turn it on in **Settings**. Before your first recording, you confirm that you'll get consent from the other people before you record them.

**Do I need everyone's consent?**
Yes. Recording laws vary by place, and many require everyone's consent. Tell people you're recording, and follow your company's policy. Bottle records your mic and the other side of the call.

**Where do recordings go?**
They go to a Unity Catalog volume that you choose. In the recording settings, you have two options:
- Enter a volume path, such as `/Volumes/catalog/schema/volume`.
- Enter a `catalog.schema`, and Bottle can create a volume named `bottle_audio` there.

After the upload succeeds, Bottle deletes the temporary audio from your Mac.

**What do I get from a recording?**
You get a transcript (written by Gemini) and notes (written by Claude). By default, the notes include action items for each speaker. Both models run through your AI Gateway. Find them under **Recordings** in the hub.

**A recording didn't upload. Did I lose it?**
No. It stays on your Mac, and **Recordings** lists it right away as only on this device, the moment you press Stop. Upload it from there to get the transcript and notes. **Remove** deletes the copy on your Mac; if the recording never reached Databricks, that's the only copy, so Remove warns you it deletes it for good.

## Your data

**How do I delete my data?**
- **Wipe** in **Settings** deletes local history older than 30 days, older than 90 days, or everything.
- **Forget** deletes one conversation, or all of them.

Both work offline and take effect right away.

**Does that delete it in Databricks too?**
Not all of it. Wipe and Forget clear what's on your Mac. When Bottle can't delete a Genie conversation itself, it gives you a **Delete in Genie One** link. Recordings stay in your volume until you delete them. Nobody can delete admin audit records.

**How do I uninstall Bottle?**
Drag Bottle to the Trash. To remove its data too, delete these two folders:
- `~/Library/Application Support/com.databricks.bottle`
- `~/Library/Application Support/Bottle`

Your `bottle-*` profile stays in your Databricks CLI config. Remove it there if you want.

## Troubleshooting

- **Right Option does nothing:** turn on Input Monitoring and Accessibility, then quit and reopen Bottle.
- **Words don't appear where I type:** check Accessibility, and click into a text field before you talk. Or use **Copy** on the pop-up.
- **"Could not transcribe":** hold the key and try again. Check your mic in macOS Sound settings.
- **"Transcription of this language is not available":** set **Dictation engine** to **Databricks**.
- **"This sounded like an instruction to Bottle. Nothing was sent.":** a safety check flagged your words as a command. Rephrase them as a plain question.
- **Genie returned an error:** the card shows a reason and a code. Bottle never retries on its own, so a question is never sent twice. Check that the Genie space opens in your browser, then ask again.
- **"Model not reachable" note:** you still got an answer, straight from Genie on your home workspace. Check that the model chosen in **Settings → Workspace** is available to you.
- **Sign-in stuck:** Bottle waits 5 minutes for the browser. Use **Open the browser again**, or **Cancel** and start over.
- **A workspace's sign-in expired mid-question:** Bottle opens that workspace's browser sign-in for you and retries the call once on its own; finish the sign-in in the browser and the answer continues.

## What's next

**Windows or Linux?**
Windows is in progress. An early installer exists where dictation works with Right Alt, but Ask Genie by key and call recording are Mac only for now. Linux is planned.

**What's coming?**
Bottle will help your **team remember**. The know-how that makes data useful is often said in a call and then lost, for example "an active user is anyone who logged in this month". Bottle will turn that into governed context in Unity Catalog, and **only after you review and approve it**. After that, every Genie answer improves, yours and your teammates'.

**How do I update?**
Download the latest release from GitHub and replace the app in Applications. Your settings, history and permissions stay.

**How do I report a bug?**
Open a GitHub issue with your Bottle version, your macOS version, and what you expected to happen.
