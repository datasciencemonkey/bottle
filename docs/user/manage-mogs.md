# Manage your MOGs

The hub lists every MOG that you define. You can define any number of MOGs.

## Actions

- **Create MOG:** type a name and a home profile, then select Create MOG. With no name, Bottle uses "MOG 1", "MOG 2" and so on.
- **Edit:** change the name, the home profile or the persona instructions (2,000 characters at most). Select Save. Esc cancels.
- **Duplicate:** Bottle makes a copy with the name "<name> copy". The copy has no per-MOG key.
- **Start:** Bottle shows the MOG on your desktop.
- **Stop:** Bottle closes the MOG.
- **Delete:** Bottle asks first. Delete removes only the settings of the MOG. Its memory and history stay until you use Wipe in Settings.

## How many MOGs can run

8 MOGs can run at the same time. If 8 MOGs run and you start one more, the hub shows "8 MOGs are running. Stop one to start <name>." Stop a MOG, then start the new one.

## Keyboard

Use Tab to move between the buttons and the fields. Each button names its MOG, for example "Start Sales". A screen reader reads the running count and each message.

## Talk with no profile

Dictation works with no Databricks CLI profile and no sign-in. Bottle then decides offline: a sentence with a question mark, or one that starts with a question word (what, when, where, which, who, why, how), is a question. All other speech is dictation.

Every utterance is dictation first. Bottle pastes your text at the cursor. If the paste is refused, the pop-up beside the MOG shows the text with **Copy**.

After each dictation, the pop-up also shows **Ask Genie** and the hint "Hold ⌥G to ask Genie". Move the pointer over the pop-up to keep it open and read the text. Nothing goes to Genie until you press **Ask Genie**. When you press it, Bottle removes hidden characters, HTML, code fences and link addresses from the text. The card then shows the exact text that goes to Genie. A text over 1,000 characters is cut at a word edge, and the card says so. The press sends the text straight to Genie One in the workspace of your home profile. The card shows "Asking Genie", then the answer. If Genie does not answer, the card shows the reason and a code. Bottle does not send the question again. If you ignore the pop-up, it fades.

- **No home profile:** a press on Ask Genie shows your Databricks CLI profiles. Select one. Bottle saves only its name as the home profile of the MOG, then sends the text. Bottle never picks a profile for you.
- **⌥G during the hold:** hold the talk key, tap G, then release. Bottle does not paste. It sends the exact text to Genie at once, with no preview. With no home profile, the profile list shows first. Not yet on Windows.
- **History:** Bottle saves each question that it sent to Genie, with the answer and its key numbers, in the encrypted store on this device. Nothing goes to the cloud. If the store is not available, the card still shows the answer, and nothing is saved. Use Forget or Settings → Wipe to delete it.
