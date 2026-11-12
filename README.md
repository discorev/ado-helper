# ado helper

`ado` is a small macOS and Linux command-line tool for Azure DevOps pull-request review, work-item automation and read-only pipeline diagnostics. It stores PATs in the native credential store, emits JSON for review data, and uses the SSH identity already configured for Git clones. It never passes a PAT to Git.

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

Replace the placeholders with the organisation name from each ADO URL; one profile per organisation lets you use a different identity for each. Select **Code → Read & write** (`vso.code_write`), **Pull Request Threads → Read & write** (`vso.threads_full`, for comments), **Work Items → Read & write** (`vso.work_write`), **Identity → Read** (`vso.identity`, to resolve `--reviewer`/`--add-reviewer` emails and names to identity IDs), **Build → Read** (`vso.build`, for pipeline commands), and **Test Management → Read** (`vso.test`, for pipeline test discovery and results) under custom scopes (use **Show all scopes**). Creating or updating pull requests requires **Code → Read & write**; the Pull Request Threads scope covers comments only. Existing PATs created with **Code → Read** or without work-item write access may need to be regenerated before using the new PR mutations or `ado wi` commands. Existing PATs without **Build → Read** and **Test Management → Read** need to be regenerated with those scopes before using pipeline diagnostics; save the replacement with `ado auth update NAME`. No Azure subscription login or Azure CLI configuration is used. Setup verifies the account identity; it cannot attest to the scope selections you made in the browser. [Microsoft scope reference](https://learn.microsoft.com/en-us/azure/devops/integrate/get-started/authentication/oauth?view=azure-devops).

Profile metadata is stored in `~/.config/ado/profiles.json`; PATs are stored only in the macOS login Keychain or Linux Secret Service. On macOS, the native executable is trusted for its own items; other executables remain subject to Keychain approval. Install before adding profiles. Rebuilding or moving the executable can require renewed Keychain approval on macOS.

`ado help` prints the full usage. Add `--help` or `-h` after a group or command (for example `ado pr auto-complete --help` or `ado pr --help`) to print only its usage; the command itself is not run.

Review commands accept a full Azure DevOps PR URL, a PR number, or no target. A number uses `--profile NAME` when given and otherwise derives the organization from the current Azure Git remote. An omitted target finds the PR for the current branch. Examples:

```sh
ado pr query --reviewer me --top 25
ado pr show 123 --profile work
ado pr create --title "Add search" --reviewer ada@example.com --work-item 456 \
  --tags "api,ready"
ado pr update 123 --ready --add-reviewer grace@example.com --tags "api,approved"
ado pr update 123 --add-required-reviewer nick@example.com
ado pr update 123 --add-work-item 456 --add-work-item 789
ado pr open 123
ado pr threads https://dev.azure.com/ORG/PROJECT/_git/REPO/pullrequest/123
ado pr vote 123 --vote approve
ado pr update 124 --target main
ado pr complete 123 --merge-strategy squash --delete-source-branch
ado pr create --title "Add search" --work-item 456 --auto-complete --merge-strategy squash
ado pr auto-complete 123
ado pr auto-complete 123 --clear
ado pr checks 123 --wait
ado pr thread-status 123 --thread-id 456 --status fixed
ado pr changes 123 --iteration 4
ado pr clone 123
ado pr diff 123 --directory ~/Developer/ado-reviews/custom
```

`pr query` lists pull requests in the current repository. Its status defaults to `active` and `--top` defaults to 100; `--creator me` and `--reviewer me` use the authenticated profile identity. `--source` and `--target` accept branch names with or without the `refs/heads/` prefix.

`pr create` derives the organisation, project and repository from the current Azure Git remote. The source defaults to the current branch and the target defaults to the repository's default branch. The source must already exist on the remote, and when it is the current branch its remote tip must equal local `HEAD`; `ado` never pushes. Repeat `--reviewer` and `--work-item` to add more than one. A reviewer already on the PR is left unchanged, keeping any vote and required flag; `--force` (requires `--reviewer`) re-adds every listed reviewer with vote 0 instead. The final readback confirms each requested reviewer is present. `--tags` is a comma-separated PR-label list.

`--merge-strategy`, `--delete-source-branch`, and `--keep-source-branch` are only accepted together with `--auto-complete`. Auto-complete is set last, after the new PR's labels are added and its reviewers are confirmed present, so a failed reviewer or label follow-up never leaves auto-complete set on an incomplete pull request. If setting auto-complete fails, the PR is still printed and the command exits non-zero, saying auto-complete was not confirmed; check with `ado pr show`, since a request that timed out may still have been applied.

`pr update` accepts the same URL, number or current-branch target resolution as other PR commands and requires at least one field option. Use `--draft` or `--ready` to change draft state, `--status active|abandoned` to change status, and repeat `--add-reviewer` as needed. Reviewers accept an email, unique name or identity GUID; anything other than a GUID is resolved through the organisation's identity service and must match exactly one identity. A reviewer already on the PR is left unchanged, keeping any vote and required flag; `--force` (requires `--add-reviewer`) re-adds every listed reviewer with vote 0, resetting both. The final readback confirms each requested reviewer is present. Supplying `--tags` replaces the complete PR-label set; use an empty value to clear it. `--target BRANCH` retargets the PR (for example after the PR it was stacked on has merged). It reads the current target first and does nothing if the PR already targets that branch, because Azure DevOps returns 400 for a same-target retarget. After a retarget it sends `mergeStatus: queued` to make Azure DevOps recompute the preview merge against the new target; check `mergeStatus` with `ado pr show` before completing. Repeat `--add-work-item ID` to link work items to an existing PR; Azure DevOps ignores `workItemRefs` on an update, so each link is added from the work item side as an `ArtifactLink` relation, and a work item already linked to the PR is left unchanged. The final readback of the PR's linked work items confirms each requested ID is present.

Repeat `--add-required-reviewer` to add a reviewer as required, or to switch an existing optional reviewer to required; it accepts the same identity forms as `--add-reviewer`. It sends the reviewer's current vote back unchanged, so it never resets a vote already cast, and re-reads the PR afterwards to confirm `isRequired: true` and the unchanged vote. A reviewer may not be passed to both `--add-reviewer` and `--add-required-reviewer`.

`pr complete` completes the PR using `--merge-strategy no-fast-forward|squash|rebase|rebase-merge` (default `no-fast-forward`). It keeps the source branch unless `--delete-source-branch` is given. It refuses a draft PR, a PR that is not active, and a PR whose `mergeStatus` is not `succeeded`. It pins the merge to the PR's current source commit (`lastMergeSourceCommit`), so a push after you looked is rejected rather than merged. Azure DevOps may finish the merge asynchronously; confirm with `ado pr show`. If completion fails and an enabled, blocking branch policy has not passed, the error names the blocking policies and their status and suggests `ado pr checks N --wait`. If the PR is still active after completing (Azure DevOps rebuilt the merge commit and re-ran required builds), run `ado pr checks N --wait` and retry.

`pr checks` reports branch-policy status: `ado pr checks [TARGET] [--wait] [--timeout SECONDS] [--interval SECONDS]`. Each enabled policy evaluation becomes one entry in `checks`; disabled policies are omitted. Only enabled, *blocking* evaluations count toward `summary` and the overall `state`: `approved` or `notApplicable` count as passed, `rejected` or `broken` count as rejected, and anything else (`queued`, `running`, or an unrecognised status) counts as pending; an expired build keeps whatever status Azure DevOps reports for it, which is usually one of the pending statuses. `state` is `rejected` if any blocking evaluation is rejected, else `pending` if any is still pending, else `passed`. The command exits `0` for `passed`, `2` for `rejected`, and `3` for `pending` (or, with `--wait`, for `timedOut`); a generic error still exits `1`. With `--wait`, it polls every `--interval` seconds (default 15, minimum 5) for up to `--timeout` seconds (default 1800, must be greater than zero), printing a stderr line each time a check's status changes, and returns as soon as the state is no longer pending so a blocking rejection is reported immediately. Run `ado pr checks 123 --wait` as a background task so an agent is woken only once checks settle, instead of polling `ado pr show` in a loop. It needs only the **Code (Read)** scope, which the **Code (Read & write)** scope set up above already covers.

`pr auto-complete` sets auto-complete as the authenticated user, so Azure DevOps completes the PR itself once policies pass, instead of completing it immediately like `pr complete`. `--merge-strategy` is case-insensitive and also accepts the hyphenated `pr complete` forms (default `noFastForward`). Unlike `pr complete`, it deletes the source branch by default; pass `--keep-source-branch` to keep it. Other existing completion options, such as work-item transition and the merge commit message, are sent back unchanged so they are not reset. `--clear` cancels auto-complete and cannot be combined with the other options. Either way, it reads the PR back afterwards and confirms the change took effect.

`pr vote` sets the authenticated user's vote: `--vote approve` (10), `approve-with-suggestions` (5), `reset` (0), `wait` (-5), or `reject` (-10). It reads the current reviewer entry, if any, and sends `isRequired` back unchanged so voting never drops a required-reviewer flag. `pr thread-status` sets a thread's resolution with `--thread-id N --status active|fixed|wontfix|closed|bydesign|pending`; get thread IDs from `ado pr threads`.

Clone and diff use only validated Azure SSH URLs, fetch the exact source and target refs, and verify their reviewed commit IDs. By default review checkouts live under `~/Developer/ado-reviews/ORG/REPO-pr-ID`. `ado` records their identity in `.git/ado-review.json`, refuses unmarked or dirty directories, disables hooks while refreshing, and disables external diff and text-conversion drivers. Cross-organization forks are rejected; same-organization forks use their separately validated source SSH URL.

For a repository you do not have locally, pass the full PR URL to `ado pr clone`; no local checkout is required. Branch discovery uses the configured upstream remote (or an unambiguous Azure remote). Use the full URL for a fork PR so its target repository is explicit.

To respond to a reviewer, reply inside their existing thread; get thread IDs from `ado pr threads`:

```sh
ado pr comment 123 --thread-id 456 --body-file /tmp/reply.txt
```

The parent defaults to the thread's first comment: Azure DevOps shows a thread as a flat list, and the web UI's reply box attaches replies to that first comment by convention. Pass `--parent-comment-id` to target a specific comment instead. The command verifies the posting identity and reads the new reply back; it needs no commit, since replies are not anchored to a reviewed revision. `--top-level` is only for starting a new PR-wide discussion; never use it to answer an existing comment.

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

Top-level comments require the current source commit but no file or iteration context. Mixing `--top-level` with inline anchor options is rejected. Both modes verify the posting identity and read back the new thread; top-level verification checks the exact body, author and absence of a file anchor. If submission or verification is uncertain, inspect `ado pr threads` before retrying to avoid duplicates. Authentication stays inside `ado` using its native credential store; no external token extraction is needed.

Edit an existing comment authored by the authenticated account using its thread and comment IDs from `ado pr threads`:

```sh
ado pr edit-comment 123 --thread-id 456 --comment-id 1 \
  --body-file /tmp/revised.txt --expected-body-file /tmp/original.txt
```

The expected-body file must match the current saved text exactly, apart from one optional trailing newline, so an edit based on stale text is rejected. The command patches only that comment's content and verifies the saved result. It does not create a new thread or change its status. The expected-text check happens immediately before submission; it is not a server-side atomic compare-and-swap.

Delete a misplaced comment of your own with the same expected-body check as edit, to make sure you are removing the comment you think you are:

```sh
ado pr delete-comment 123 --thread-id 456 --comment-id 1 \
  --expected-body-file /tmp/original.txt
```

Only the authenticated user's own, undeleted comment may be deleted. The command verifies that the comment is gone from the immediate readback.

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
ado wi fields --type Bug
ado wi create --type Bug --title "Add search" --field Custom.Severity=High
ado wi create --type Task --title "Add search endpoint" --link parent:100 --link related:101
ado wi update 123 --link successor:130 --remove-link related:101
```

`wi query` requires exactly one of `--wiql`, `--wiql-file`, or `--search`; `--top` defaults to 100. Search covers title and description and orders by the most recently changed work items. Description files are sent as-is, including HTML. `wi update` requires at least one field option, and `--tags` replaces the complete tag value. `--area` and `--iteration` set `System.AreaPath` and `System.IterationPath` on `wi create` and `wi update`; pass the full backslash-separated path, e.g. `--area 'Project\Team'`. `wi create --priority` accepts 1 to 4. `wi open` and `pr open` print the web URL before launching the platform browser.

Process-specific fields are set by reference name: `wi fields --type TYPE` lists every field for a work item type with its `referenceName` and any `allowedValues`; pass `--field REF=VALUE` (repeatable) to `wi create` or `wi update`, e.g. `--field Custom.Severity=High`. Values are sent as-is and validated by Azure DevOps. `--field` splits on the first `=`, and counts as a field option for `wi update`.

Work items link to each other with `--link TYPE:ID` (repeatable) on `wi create` and `wi update`, and unlink with `--remove-link TYPE:ID` (repeatable, `wi update` only). `TYPE` is one of `parent`, `child`, `related`, `predecessor` (the item this one depends on), or `successor`; an unknown type or a non-positive `ID` is a parse error, and passing the same `TYPE:ID` to both `--link` and `--remove-link` is also a parse error. `wi update` is idempotent: adding a link that already exists is a no-op (an item is never linked twice), and removing one that doesn't exist fails with an error naming the missing link, e.g. "Work item 7 has no predecessor link to 12." `--link` and `--remove-link` both count as field options for `wi update`.

## Pipeline diagnostics

Pipeline commands are read-only and use the Build and Test REST APIs. Like `wi`, they derive the project from the current Azure Git remote unless `--project NAME` is supplied, and accept `--profile NAME`. A run target is a positive build ID or a full `https://dev.azure.com/acme/Example%20Project/_build/results?buildId=123` URL. A URL supplies its organization and project without requiring a Git checkout; a conflicting `--project` or profile organization is rejected.

```sh
ado pipeline list --name "example-*"
ado pipeline runs --pipeline CI --branch main --top 25
ado pipeline runs --status completed --result failed --project "Example Project" --profile work
ado pipeline show 123
ado pipeline log 123 --failed --tail 100
ado pipeline log 123 --log-id 7
ado pipeline tests 123
ado pipeline tests 123 --all
ado pipeline open 'https://dev.azure.com/acme/Example%20Project/_build/results?buildId=123'
```

`pipeline list --name` uses Azure DevOps' definition-name filter (including `*` wildcards). `pipeline runs` is newest first (queue time descending), with `--top` defaulting to 25. `--pipeline` accepts a definition ID or an exact, case-insensitive name; duplicate names must be disambiguated by ID. Branch names accept an optional `refs/heads/` prefix. Status values are `none`, `inProgress`, `completed`, `cancelling`, `postponed`, `notStarted`, and `all`; result values are `none`, `succeeded`, `partiallySucceeded`, `failed`, and `canceled` (case-insensitive).

`pipeline show` includes a compact timeline with stage/job/task parent IDs, error/warning issues and a test-run existence/count hint. Intermediate phase records are omitted, with their children linked to the nearest retained parent. `pipeline log` requires exactly one of `--log-id` or `--failed`; it prints raw text rather than JSON. Failed task logs have stage/job/task headers; a failed task without an available log gets a `(no log available)` header, and a build with no failed tasks prints nothing on stdout and a note on stderr. `--tail N` keeps the last N lines of each log, preserving line endings; 0 keeps no log lines. `pipeline tests` returns test-run counts from the Test API's run statistics and paged failed results; `--all` includes all outcomes. `notExecuted` counts the API's `NotExecuted` outcome, rather than treating other outcomes such as `Blocked` as skipped. `pipeline open`, like other open commands, prints the web URL before launching it.

The JSON output contracts for pipelines are:

- `pipeline list`: an array of `{id, name, folder, yamlPath, repository, url}`. YAML path and repository name are `null` when unavailable (for example, a classic definition).
- `pipeline runs`: an array of `{id, buildNumber, pipeline: {id, name}, status, result, sourceBranch, sourceVersion, reason, requestedFor, queueTime, startTime, finishTime, url}`. `requestedFor` is the display name; unpopulated API fields are `null`.
- `pipeline show`: the same run object, plus `timeline: [{id, parentId, name, type, state, result, startTime, finishTime, logId, issues}]` and `tests: {exists, count}`.
- `pipeline tests`: `{buildId, runs: [{id, name, state, total, passed, failed, notExecuted, results}]}`. Each result is `{id, testName, automatedTestName, outcome, errorMessage, stackTrace, durationInMs, testRun: {id, name}}`. Run counts always cover all outcomes, even when only failed results are returned.
- `pipeline log`: raw log text, not JSON. `pipeline open`: the web URL as one text line, not JSON.

The JSON output contracts for pull-request mutations and queries are:

- `pr query`: an array of `{id, title, status, isDraft, creator, source, target, reviewers, url}`. `creator` and each reviewer's `identity` use the unique name, falling back to display name or `null`; reviewer entries also include `vote`.
- `pr create` and `pr update`: `{id, url, fields}`, where `fields` is Azure DevOps' complete pull-request object.
- `pr complete`: Azure DevOps' complete pull-request object as returned by the update, which may still show `status: active` while the merge finishes.
- `pr auto-complete`: Azure DevOps' complete pull-request object, read back after the change to confirm it took effect.
- `pr checks`: `{pullRequestId, url, state, summary: {passed, rejected, pending}, checks}`, where each entry in `checks` is `{name, type, blocking, status, expired, buildId, buildUrl}`; `buildId` and `buildUrl` are `null` for a non-build policy.
- `pr vote`: Azure DevOps' complete reviewer object (`IdentityRefWithVote`), including the resulting `vote` and `isRequired`.
- `pr thread-status`: Azure DevOps' complete thread object, including the resulting `status`.
- `pr comment --thread-id`: Azure DevOps' complete comment object.
- `pr delete-comment`: `{pullRequestId, threadId, commentId, deleted}`.

The JSON output contracts for work items are:

- `wi query`: an array of `{id, type, title, state, assignedTo, areaPath, iterationPath, tags, url}`. `assignedTo` is the identity's unique name, falling back to display name or `null`; `tags` is an array.
- `wi show`, `wi create`, and `wi update`: `{id, url, fields, customFields, relations}`, where `fields` is Azure DevOps' complete fields object, `customFields` holds every field whose reference name starts with `Custom.`, keyed by reference name (an empty object when none), and `relations` is an array of `{type, rel, id, url, name}` (an empty array when none) — `type` is the friendly link name (`parent`, `child`, `related`, `predecessor`, `successor`) or `null` for other relation kinds such as pull request links, `id` is the linked work item's ID or `null`, `url` is the web URL for a work-item link and the raw relation URL otherwise, and `name` is `attributes.name` when Azure DevOps provides one (e.g. `"Pull Request"`), otherwise `null`.
- `wi comments`: an array of `{id, author, createdDate, text}`; `wi comment` returns one object in the same shape.
- `wi open` and `pr open`: the web URL as a single text line rather than JSON.
- `wi fields`: an array of `{name, referenceName, allowedValues}` for the given work item type; `allowedValues` is an empty array when the field has none.

For automation and agents, successful PR data commands and `wi query`, `show`, `comments`, `comment`, `create`, `update`, and `fields` output JSON on stdout. Diagnostics go to stderr and failures return a nonzero exit status. Authentication and help remain human-readable and require an interactive terminal where secrets or confirmation are involved. Never parse diagnostics for data, and never place tokens in arguments or environment variables.

The offline suite exercises mock ADO responses, credential-store failures and real temporary Git repositories. The Linux container suite additionally exercises a real Secret Service round-trip. Live ADO permissions, organisation policy and credential-store approval must be verified during interactive setup; tests do not write real credentials or mutate live pull requests. This version reads and updates PR metadata, labels, reviewers, work-item links, changes and threads, reads and updates work items and their links to each other, reads branch-policy evaluation status, and reads pipeline definitions, builds, timelines, logs and test results without queuing or changing runs.

## License

MIT. See [LICENSE](LICENSE).

JSON output is stable data, but consumers should parse it rather than depend on whitespace or slash-escaping details.
