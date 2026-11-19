# Install the Databricks CLI

Bottle uses the Databricks CLI to connect to a workspace.
Bottle does not include the CLI in its installer.

If Bottle finds no CLI, or a CLI older than version 1.17, the hub shows the step "Install the Databricks CLI".
This step comes before the workspace setup.
The step shows the source, the version and the install folder.

- Source: https://github.com/databricks/cli/releases
- Version: 1.26.0
- Install folder on macOS: `~/Library/Application Support/Bottle/bin`
- Install folder on Windows: `%LOCALAPPDATA%\Bottle\bin`

## Install the CLI

1. Read the source, the version and the install folder.
2. Select "Install the Databricks CLI".

Bottle downloads the CLI only after you select "Install the Databricks CLI".
Bottle checks the SHA-256 checksum of the download before it unpacks the CLI.
If the checksum is not correct, Bottle stops the install and deletes the download.
The install needs no admin rights. It does not change your PATH.

When the install is complete, the hub shows the path of the CLI that Bottle uses.

## Not now

Select "Not now" to close the step.
The hub then shows "No workspace connected".

## An older CLI

If you have a CLI older than version 1.17, Bottle does not change it.
Bottle installs its own copy in the install folder and uses that copy.
Settings → Databricks CLI shows the path of the CLI that Bottle uses.
