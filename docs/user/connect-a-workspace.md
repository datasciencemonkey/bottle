# Connect a workspace

Bottle connects to a Databricks workspace through a Databricks CLI profile.
"Connect a workspace" makes a new OAuth profile for you. You do not need a terminal.

When Bottle has no usable profile at first run, the hub opens "Connect a workspace" at once.
You can also open it from the hub at any time.

## Steps

1. Open your workspace in the browser and copy the URL.
2. Paste the URL in "Workspace URL" and select Next.
   Bottle keeps only the host, for example `https://<name>.cloud.databricks.com`.
   Bottle accepts only Databricks workspace hosts.
3. Bottle proposes the profile name `bottle-<workspace>`. You can change it.
   The name must not exist already.
4. Optional: open "Use a custom OAuth app" and type the OAuth client ID that your admin gave you.
5. Select "Sign in". Bottle starts `databricks auth login` and your browser opens.
6. Finish the sign-in in your browser. Bottle waits up to 5 minutes.
   - Select "Open the browser again" if you closed the browser page.
   - Select Cancel to stop the sign-in.
7. Bottle checks the new profile with your workspace. Then the hub shows "Connected".

If the check fails, the hub shows the reason and one action, for example "Try again".

## Your other profiles

Bottle never changes a profile that exists.
If a personal access token or service principal profile exists for the same workspace, Bottle tells you.
Bottle does not use or change it. Bottle makes a new `bottle-*` OAuth profile.

Bottle never reads or stores your token. The Databricks CLI keeps the sign-in in its own store.
