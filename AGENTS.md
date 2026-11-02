# Ado Helper

This project exists so agents can use Azure DevOps (ADO) without needing the user to be logged into the `az` command-line that may have access to the wider Azure platform.

Agents are instructed to use the installed `ado` command when interacting with ADO; this allows a single built, signed binary to be used that carries permission to access the required keychain entries across all agent sessions.

## The definition of done

For a change to be completed, it means the work is committed, pushed and installed. Never end a task with uncommitted changes unless the user explicitly asks you not to commit.

Before making changes, agree with the user which branch to use (main or a feature branch) and confirm that installing is OK. Installing is required for agents to be able to find and use the commands as they do not read the source or build it themselves.
