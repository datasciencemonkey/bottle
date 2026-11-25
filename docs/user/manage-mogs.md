# Manage your MOG and its workspaces

Bottle runs one MOG. Each connected Databricks CLI profile is a workspace of that MOG.
You see every workspace in **Settings → Workspace**, and you turn each one on or off there.

## Settings → Workspace

The list shows your home workspace first, then every other workspace: a colour swatch, a name,
the workspace address, and a sign-in status (Signed in, Expired, Not checked yet, Not supported,
or Not in your CLI config).

The home row has a **Home** badge and no switch; it says **Always on**.
Every other row has a **Make home** button and a switch, **Available to your MOG**.

## Turn a workspace on or off

Use the **Available to your MOG** switch on a workspace row. On sends your questions to that
workspace when a question needs more than your home workspace. Off keeps it out.

A new connection starts off. Turn it on to let your MOG use it.

Some rows can't be turned on or made home, for example a profile type Bottle does not support, or
a profile missing from your `~/.databrickscfg`. Their controls are greyed out, and the row says why.

## Make home

Select **Make home** on a workspace row. Bottle asks "Make `<name>` your home? Plain questions go
there first." Select **Make home** again to confirm, or **Cancel**.

Your home workspace always stays on. When you hold the talk key and tap G (⌥G) to ask a plain
question, Bottle asks your home workspace first. It asks your other switched-on workspaces only
when your home workspace has no data or cannot answer, or when your question names another
workspace by name.

If you have no home workspace yet, the Workspace section shows "Choose a home workspace" with a
**Choose…** button. Select it to open "Choose your home workspace": pick one from the list and
select **Use as home**, or select **Not now** to decide later.

## Refresh, sign in, and renaming

Select the refresh icon beside "Workspace" to recheck every workspace's sign-in status.

A workspace with **Expired** status shows a **Sign in…** button. Select it to sign in again in
your browser. You can also run the command shown under "Or run in Terminal:" and select **Copy**
to copy it.

Open the **…** menu on a row for **Rename** or **Colour**.

## Connect another workspace

Select **Connect another workspace…** at the bottom of the Workspace section, then **Connect…**.
This opens the same "Connect a workspace" flow as first run: paste your workspace URL, then set a
**Profile name** (Bottle saves it as a Databricks CLI profile in `~/.databrickscfg`), then select
**Sign in**.

## Memories and forgetting

Your MOG's memories are on the **Memory** page in the hub. To delete them, select **Forget memory…**,
then **Forget** in the dialog. This deletes all sessions, transcripts, notes and Genie history of
your MOG on this Mac. You cannot undo this.
