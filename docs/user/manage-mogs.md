# Manage your MOG and its workspaces

Bottle runs one MOG. Each connected Databricks CLI profile is a workspace of that MOG.
You see every workspace in **Settings → Workspace**, and you turn each one on or off there.

## Settings → Workspace

The list shows your home workspace first, then every other workspace: a colour swatch, a name,
a muted host, and a sign-in status (Signed in, Expired, Not checked yet, Not supported, or Not in
your CLI config).

The home row has a **Home** badge and no switch; it says **Always on**.
Every other row has a **Make home** button and a switch, **Available to your MOG**.

## Turn a workspace on or off

Use the **Available to your MOG** switch on a workspace row. On sends your questions to that
workspace when a question needs more than your home workspace. Off keeps it out.

A new connection starts off. Turn it on to let your MOG use it.

Some rows have no switch to turn on and no **Make home** button, for example a profile type
Bottle does not support, or a profile missing from your `~/.databrickscfg`. The row says why.

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

Your MOG's memories live on the **Memory** page in the hub. Select **Forget memory…** there, then
confirm with **Forget** in the dialog, to delete them.

## Could not verify

The following claims from the stale version of this page could not be verified against the
int-0110 screen text and are dropped rather than guessed:

- Exact keyboard navigation behaviour (Tab order) across the Workspace list.
- Whether a screen reader announces a running count, since that applied to the old multi-MOG hub
  and has no equivalent in the one-MOG Settings → Workspace view.
