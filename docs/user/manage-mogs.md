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

A question needs a home profile. If the MOG has none, its card shows your text and "Pick a profile to ask Genie." Nothing goes to Genie.

- **Pick a profile:** the card lists your Databricks CLI profiles. Select one. Bottle saves only its name as the home profile of the MOG. Bottle never picks a profile for you.
- **Insert as text:** if you meant dictation, Bottle inserts the text, or keeps it on the card when insertion is off.
- **Copy:** the text stays on the card for you to copy.
