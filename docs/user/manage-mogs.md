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

If your text looks like a question, the pop-up also shows **Ask Genie**. Move the pointer over the pop-up to keep it open and read the text. Nothing goes to Genie until you press **Ask Genie**. The press sends the exact text that you see. If you ignore the pop-up, it fades.

- **No home profile:** a press on Ask Genie shows your Databricks CLI profiles. Select one. Bottle saves only its name as the home profile of the MOG, then sends the text. Bottle never picks a profile for you.
- **Shift with the talk key:** this still asks at once, with the 1.5 s preview that you can cancel.
