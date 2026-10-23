# ado helper

`ado` is a small macOS and Linux command-line tool for Azure DevOps pull-request review and work-item automation. It stores PATs in the native credential store, emits JSON for review data, and uses the SSH identity already configured for Git clones. It never passes a PAT to Git.

Requires Rust 1.91 or later. Build and test the offline-capable code with:

```sh
cargo build
cargo test
```

On Linux, a Secret Service provider such as GNOME Keyring must be installed, running, and unlocked. There is no plaintext credential fallback. To run the full suite in a Linux container, including a real Secret Service save/read/remove round-trip:

```sh
./scripts/test-linux.sh
```

To install after reviewing the script:

```sh
./scripts/install.sh
```

This builds a release executable and installs it at `~/.local/bin/ado` with owner-only write permission. Set `ADO_INSTALL_DIR` to choose another bin directory. On macOS, the script uses the sole valid code-signing identity in Keychain and verifies the signature before replacing the installed binary. The identifier remains `dev.ollies.ado-helper`. If there are zero or multiple valid identities, it stops and asks you to select one explicitly:

```sh
ADO_CODESIGN_IDENTITY="Apple Development: Your Name (TEAMID)" ./scripts/install.sh
```

You can also supply the certificate SHA-1 shown by `security find-identity -v -p codesigning`. Use the same signing identity for future builds. Changing from ad-hoc signing to a certificate can require a one-time Keychain approval; subsequent builds retain the certificate-based designated identity. Ad-hoc signing is available only by explicitly setting `ADO_CODESIGN_IDENTITY=-` and can require approval again after each rebuild.

Run the installer regression tests without accessing Keychain or compiling Rust with `python3 scripts/test-install.py`.

Create a profile interactively:

```sh
ado auth add work --org https://dev.azure.com/YOUR-ORG
ado auth add personal --org https://dev.azure.com/YOUR-OTHER-ORG
ado auth status
ado auth update work
```

The command opens the organization PAT form unless `--no-browser` is supplied, explains the required scopes, reads the PAT with hidden terminal input, verifies its identity, asks before saving, then stores it in macOS Keychain or Linux Secret Service. Profiles map organizations to identities; profile files contain no tokens. `auth status` only reads local profile metadata unless `--check` is supplied. `auth remove NAME` removes local profile and credential-store data; it does not revoke the PAT in Azure DevOps.

Replace the placeholders with the organisation name from each ADO URL; one profile per organisation lets you use a different identity for each. Select **Code → Read & write** (`vso.code_write`), **Pull Request Threads → Read & write** (`vso.threads_full`, for comments), and **Work Items → Read & write** (`vso.work_write`) under custom scopes (use **Show all scopes**). Creating or updating pull requests requires **Code → Read & write**; the Pull Request Threads scope covers comments only. Existing PATs created with **Code → Read** or without work-item write access may need to be regenerated before using the new PR mutations or `ado wi` commands. No Azure subscription login or Azure CLI configuration is used. Setup verifies the account identity; it cannot attest to the scope selections you made in the browser. [Microsoft scope reference](https://learn.microsoft.com/en-us/azure/devops/integrate/get-started/authentication/oauth?view=azure-devops).

Profile metadata is stored in `~/.config/ado/profiles.json`; PATs are stored only in the macOS login Keychain or Linux Secret Service. On macOS, the native executable is trusted for its own items; other executables remain subject to Keychain approval. Install before adding profiles. Rebuilding or moving the executable can require renewed Keychain approval on macOS.

Review commands accept a full Azure DevOps PR URL, a PR number, or no target. A number uses `--profile NAME` when given and otherwise derives the organization from the current Azure Git remote. An omitted target finds the PR for the current branch. Examples:

```sh
ado pr query --reviewer me --top 25
ado pr show 123 --profile work
ado pr create --title "Add search" --reviewer ada@example.com --work-item 456 \
  --tags "api,ready"
ado pr update 123 --ready --add-reviewer grace@example.com --tags "api,approved"
ado pr open 123
ado pr threads https://dev.azure.com/ORG/PROJECT/_git/REPO/pullrequest/123
ado pr changes 123 --iteration 4
ado pr clone 123
ado pr diff 123 --directory ~/Developer/ado-reviews/custom
```

`pr query` lists pull requests in the current repository. Its status defaults to `active` and `--top` defaults to 100; `--creator me` and `--reviewer me` use the authenticated profile identity. `--source` and `--target` accept branch names with or without the `refs/heads/` prefix.

`pr create` derives the organisation, project and repository from the current Azure Git remote. The source defaults to the current branch and the target defaults to the repository's default branch. The source must already exist on the remote, and when it is the current branch its remote tip must equal local `HEAD`; `ado` never pushes. Repeat `--reviewer` and `--work-item` to add more than one. `--tags` is a comma-separated PR-label list.

`pr update` accepts the same URL, number or current-branch target resolution as other PR commands and requires at least one field option. Use `--draft` or `--ready` to change draft state, `--status active|abandoned` to change status, and repeat `--add-reviewer` as needed. Supplying `--tags` replaces the complete PR-label set; use an empty value to clear it.

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

The command checks that the commit and iteration are still current, the change ID belongs to the requested path, the requested side exists, and the line range fits the exact reviewed file version. It creates an inline thread. For a PR-wide discussion, explicitly request a top-level comment instead:

```sh
ado pr comment 123 --top-level --body-file /tmp/comment.txt \
  --commit 0123456789abcdef0123456789abcdef01234567
```

Top-level comments require the current source commit but no file or iteration context. Mixing `--top-level` with inline anchor options is rejected. Both modes verify the posting identity and read back the new thread; top-level verification checks the exact body, author and absence of a file anchor. If submission or verification is uncertain, inspect `ado pr threads` before retrying to avoid duplicates. Authentication stays inside `ado` using its native credential store; no external token extraction is needed. There are no commands for votes, merges, or resolving threads.

Edit an existing comment authored by the authenticated account using its thread and comment IDs from `ado pr threads`:

```sh
ado pr edit-comment 123 --thread-id 456 --comment-id 1 \
  --body-file /tmp/revised.txt --expected-body-file /tmp/original.txt
```

The expected-body file must match the current saved text exactly, apart from one optional trailing newline, so an edit based on stale text is rejected. The command patches only that comment's content and verifies the saved result. It does not create a new thread or change its status. The expected-text check happens immediately before submission; it is not a server-side atomic compare-and-swap.

Work-item commands accept `--profile NAME` and `--project NAME`. The project defaults to the project in the current repository's Azure `_git` remote; if it cannot be derived, pass `--project`. The organization follows the same profile and Git-remote resolution as PR commands.

```sh
ado wi query --search "login failure" --top 25
ado wi query --wiql-file /tmp/query.wiql --project "Web App" --profile work
ado wi show 123
ado wi comments 123
ado wi comment 123 --body-file /tmp/comment.txt
ado wi create --type Bug --title "Login fails" --description-file /tmp/description.html \
  --assign-to ada@example.com --tags "auth;urgent"
ado wi update 123 --state Active --tags "auth;investigating"
ado wi open 123
```

`wi query` requires exactly one of `--wiql`, `--wiql-file`, or `--search`; `--top` defaults to 100. Search covers title and description and orders by the most recently changed work items. Description files are sent as-is, including HTML. `wi update` requires at least one field option, and `--tags` replaces the complete tag value. `wi open` and `pr open` print the web URL before launching the platform browser.

The JSON output contracts for pull-request mutations and queries are:

- `pr query`: an array of `{id, title, status, isDraft, creator, source, target, reviewers, url}`. `creator` and each reviewer's `identity` use the unique name, falling back to display name or `null`; reviewer entries also include `vote`.
- `pr create` and `pr update`: `{id, url, fields}`, where `fields` is Azure DevOps' complete pull-request object.

The JSON output contracts for work items are:

- `wi query`: an array of `{id, type, title, state, assignedTo, areaPath, iterationPath, tags, url}`. `assignedTo` is the identity's unique name, falling back to display name or `null`; `tags` is an array.
- `wi show`, `wi create`, and `wi update`: `{id, url, fields}`, where `fields` is Azure DevOps' complete fields object.
- `wi comments`: an array of `{id, author, createdDate, text}`; `wi comment` returns one object in the same shape.
- `wi open` and `pr open`: the web URL as a single text line rather than JSON.

For automation and agents, successful PR data commands and `wi query`, `show`, `comments`, `comment`, `create`, and `update` output JSON on stdout. Diagnostics go to stderr and failures return a nonzero exit status. Authentication and help remain human-readable and require an interactive terminal where secrets or confirmation are involved. Never parse diagnostics for data, and never place tokens in arguments or environment variables.

The offline suite exercises mock ADO responses, credential-store failures and real temporary Git repositories. The Linux container suite additionally exercises a real Secret Service round-trip. Live ADO permissions, organisation policy and credential-store approval must be verified during interactive setup; tests do not write real credentials or mutate live pull requests. This version reads and updates PR metadata, labels, reviewers, work-item links, changes and threads, and reads and updates work items; build logs and policy evaluations are not exposed as commands.

## License

MIT. See [LICENSE](LICENSE).

JSON output is stable data, but consumers should parse it rather than depend on whitespace or slash-escaping details.
