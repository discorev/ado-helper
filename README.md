# ado helper

`ado` is a small macOS command-line tool for Azure DevOps pull-request review. It stores PATs in the native Keychain, emits JSON for review data, and uses the SSH identity already configured for Git clones. It never passes a PAT to Git.

Requires macOS 13 or later and Swift 5.9. Build and test offline-capable code with:

```sh
swift build
swift test
```

To install after reviewing the script:

```sh
./scripts/install.sh
```

This builds a release executable and installs it at `~/.local/bin/ado` with owner-only write permission. Set `ADO_INSTALL_DIR` to choose another bin directory. The script applies an ad-hoc signature with the fixed identifier `dev.ollies.ado-helper` when `codesign` is available. An upgrade changes the executable, so macOS may ask you to authorize Keychain access again.

Create a profile interactively:

```sh
ado auth add work --org https://dev.azure.com/YOUR-ORG
ado auth add personal --org https://dev.azure.com/YOUR-OTHER-ORG
ado auth status
ado auth update work
```

The command opens the organization PAT form unless `--no-browser` is supplied, explains the required scopes, reads the PAT with hidden terminal input, verifies its identity, asks before saving, then stores it in Keychain. Profiles map organizations to identities; profile files contain no tokens. `auth status` only reads local profile metadata unless `--check` is supplied. `auth remove NAME` removes local profile and Keychain data; it does not revoke the PAT in Azure DevOps.

Replace the placeholders with the organisation name from each ADO URL; one profile per organisation lets you use a different identity for each. Select **Code → Read** and **Pull Request Threads → Read & write** under custom scopes (use **Show all scopes**). No Azure subscription login or Azure CLI configuration is used. Setup verifies the account identity; it cannot attest to the scope selections you made in the browser. [Microsoft scope reference](https://learn.microsoft.com/en-us/azure/devops/integrate/get-started/authentication/oauth?view=azure-devops).

Profile metadata is stored in `~/.config/ado/profiles.json`; PATs are stored only in the macOS login Keychain. The native executable is trusted for its own items; other executables remain subject to macOS Keychain approval. Install before adding profiles. Rebuilding or moving the executable can require renewed Keychain approval.

Review commands accept a full Azure DevOps PR URL, a PR number, or no target. A number uses `--profile NAME` when given and otherwise derives the organization from the current Azure Git remote. An omitted target finds the PR for the current branch. Examples:

```sh
ado pr show 123 --profile work
ado pr threads https://dev.azure.com/ORG/PROJECT/_git/REPO/pullrequest/123
ado pr changes 123 --iteration 4
ado pr clone 123
ado pr diff 123 --directory ~/Developer/ado-reviews/custom
```

Clone and diff use only validated Azure SSH URLs, fetch the exact source and target refs, and verify their reviewed commit IDs. By default review checkouts live under `~/Developer/ado-reviews/ORG/REPO-pr-ID`. `ado` records their identity in `.git/ado-review.json`, refuses unmarked or dirty directories, disables hooks while refreshing, and disables external diff and text-conversion drivers. Cross-organization forks are rejected; same-organization forks use their separately validated source SSH URL.

For a repository you do not have locally, pass the full PR URL to `ado pr clone`; no local checkout is required. Branch discovery uses the configured upstream remote (or an unambiguous Azure remote). Use the full URL for a fork PR so its target repository is explicit.

Inline comments require the file context and explicit review identity. Generate or inspect the iteration changes first, then pass the exact source commit, iteration, and `changeTrackingId`:

```sh
ado pr comment 123 \
  --file /Sources/App.swift --line 24 --end-line 26 --side right \
  --body-file /tmp/comment.txt \
  --commit 0123456789abcdef0123456789abcdef01234567 \
  --iteration 4 --change-id 19
```

The command checks that the commit and iteration are still current, the change ID belongs to the requested path, the requested side exists, and the line range fits the exact reviewed file version. It creates an inline thread only; there are no commands for top-level comments, votes, merges, or resolving threads.

For automation and agents, successful `pr show`, `threads`, `changes`, `clone`, `diff`, and `comment` output JSON on stdout. Diagnostics go to stderr and failures return a nonzero exit status. Authentication and help remain human-readable and require an interactive terminal where secrets or confirmation are involved. Never parse diagnostics for data, and never place tokens in arguments or environment variables.

The offline suite exercises mock ADO responses, credential-store failures and real temporary Git repositories. Live ADO permissions, organisation policy and Keychain approval must be verified during interactive setup; tests do not write real credentials or post review comments. This version reads PR metadata, changes and threads; build logs and policy evaluations are not yet exposed as commands.
