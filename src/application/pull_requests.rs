use super::{
    Application,
    output::{integer, open_browser, read_body, standardize_path, write_json, write_value},
};
use ado_core::{
    Error, Result,
    ado_client::AdoClient,
    cli::PullRequestTarget,
    comment_anchor,
    git::{
        AzureGitRemote, GitRunner, GitRunning, LocalGitContext, PullRequestGitInfo, ReviewCheckout,
        is_commit_sha, validate_ref,
    },
    keychain::CredentialStore,
    organization::{Organization, PrLocator},
    profiles::Profile,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

mod management;

struct Resolved {
    locator: PrLocator,
    profile: Profile,
    client: AdoClient,
    pull_request: Value,
}

#[derive(Clone)]
struct RepositoryIdentity {
    project: String,
    repository: String,
    project_aliases: Vec<String>,
    repository_aliases: Vec<String>,
}

impl Application {
    pub(super) fn pr_show(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
    ) -> Result<()> {
        write_json(&self.resolve(target, profile_name)?.pull_request)
    }

    pub(super) fn pr_open(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let resolved = self.resolve(target, profile_name)?;
        let repository = repository_identity(&resolved.pull_request)?;
        let url = management::pull_request_url(
            &resolved.locator.organization,
            &repository.project,
            &repository.repository,
            resolved.locator.id,
        )?;
        println!("{url}");
        open_browser(&url)
    }

    pub(super) fn pr_threads(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let resolved = self.resolve(target, profile_name)?;
        let repository = repository_identity(&resolved.pull_request)?;
        write_json(&Value::Array(resolved.client.threads(
            &repository.project,
            &repository.repository,
            resolved.locator.id,
        )?))
    }

    pub(super) fn pr_changes(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
        requested: Option<i64>,
    ) -> Result<()> {
        let resolved = self.resolve(target, profile_name)?;
        let repository = repository_identity(&resolved.pull_request)?;
        let iterations = resolved.client.iterations(
            &repository.project,
            &repository.repository,
            resolved.locator.id,
        )?;
        let iteration = requested_iteration(requested, &iterations)?;
        let changes = resolved.client.changes(
            &repository.project,
            &repository.repository,
            resolved.locator.id,
            iteration,
        )?;
        write_json(
            &json!({"pullRequestId":resolved.locator.id,"iteration":iteration,"changes":changes}),
        )
    }

    pub(super) fn pr_clone(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
        directory: Option<String>,
    ) -> Result<()> {
        let resolved = self.resolve(target, profile_name)?;
        write_value(&self.prepare_checkout(&resolved, directory)?)
    }

    pub(super) fn pr_diff(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
        directory: Option<String>,
    ) -> Result<()> {
        let resolved = self.resolve(target, profile_name)?;
        let checkout = ReviewCheckout::default();
        let result = self.prepare_checkout_with(&resolved, directory, &checkout)?;
        let diff = checkout.diff(&result)?;
        write_json(&json!({"checkout":result,"diff":diff}))
    }

    fn resolve(&self, target: PullRequestTarget, profile_name: Option<&str>) -> Result<Resolved> {
        let (locator, profile) = match target {
            PullRequestTarget::Url(raw) => {
                let l = PrLocator::parse(&raw)?;
                let p = self.selected_profile(profile_name, &l.organization)?;
                (l, p)
            }
            PullRequestTarget::Number(id) => {
                if let Some(name) = profile_name {
                    let p = self.store.profile_by_name(name)?;
                    (PrLocator::new(p.organization.clone(), None, None, id), p)
                } else {
                    let ctx = LocalGitContext::discover(&GitRunner, None)?;
                    let org = Organization::parse(&ctx.remote.organization)?;
                    let p = self.store.profile_by_organization(&org)?;
                    (PrLocator::new(org, None, None, id), p)
                }
            }
            PullRequestTarget::CurrentBranch => {
                let ctx = LocalGitContext::discover(&GitRunner, None)?;
                let org = Organization::parse(&ctx.remote.organization)?;
                let p = self.selected_profile(profile_name, &org)?;
                let token = self.keychain.read(&p)?;
                let client = AdoClient::new(org.clone(), &token)?;
                let branch = format!("refs/heads/{}", ctx.branch);
                let candidates = client.list_pull_requests(
                    &ctx.remote.project,
                    &ctx.remote.repository,
                    Some(&branch),
                )?;
                let matches: Vec<&Value> = candidates
                    .iter()
                    .filter(|v| v.get("forkSource").is_none_or(Value::is_null))
                    .filter(|v| v.get("sourceRefName").and_then(Value::as_str) == Some(&branch))
                    .collect();
                let id = if matches.is_empty() {
                    return Err(Error(
                        "No active pull request was found for the current branch.".into(),
                    ));
                } else if matches.len() > 1 {
                    return Err(Error("More than one pull request matched the current branch; pass a PR URL or number.".into()));
                } else {
                    matches[0]
                        .get("pullRequestId")
                        .and_then(integer)
                        .filter(|x| *x > 0)
                        .ok_or_else(|| {
                            Error("No active pull request was found for the current branch.".into())
                        })?
                };
                (
                    PrLocator::new(
                        org,
                        Some(ctx.remote.project),
                        Some(ctx.remote.repository),
                        id,
                    ),
                    p,
                )
            }
        };
        let client = AdoClient::new(locator.organization.clone(), &self.keychain.read(&profile)?)?;
        let pr = client.pull_request(locator.id)?;
        self.validate_pr(&pr, &locator)?;
        Ok(Resolved {
            locator,
            profile,
            client,
            pull_request: pr,
        })
    }
    fn selected_profile(&self, name: Option<&str>, org: &Organization) -> Result<Profile> {
        let p = if let Some(n) = name {
            self.store.profile_by_name(n)?
        } else {
            self.store.profile_by_organization(org)?
        };
        if !p.organization.name.eq_ignore_ascii_case(&org.name) {
            Err(Error(format!(
                "Profile '{}' belongs to {}, not {}.",
                p.name, p.organization.name, org.name
            )))
        } else {
            Ok(p)
        }
    }
    fn validate_pr(&self, v: &Value, l: &PrLocator) -> Result<()> {
        if v.get("pullRequestId").and_then(integer) != Some(l.id) {
            return Err(Error(
                "Azure DevOps returned a different pull request ID.".into(),
            ));
        }
        let r = repository_identity(v)?;
        if l.project
            .as_ref()
            .is_some_and(|e| !r.project_aliases.iter().any(|a| a.eq_ignore_ascii_case(e)))
        {
            return Err(Error(
                "Pull request project does not match the target URL or repository.".into(),
            ));
        }
        if l.repository.as_ref().is_some_and(|e| {
            !r.repository_aliases
                .iter()
                .any(|a| a.eq_ignore_ascii_case(e))
        }) {
            return Err(Error(
                "Pull request repository does not match the target URL or repository.".into(),
            ));
        }
        Ok(())
    }
    fn prepare_checkout(
        &self,
        r: &Resolved,
        path: Option<String>,
    ) -> Result<ado_core::git::ReviewCheckoutResult> {
        self.prepare_checkout_with(r, path, &ReviewCheckout::default())
    }
    fn prepare_checkout_with(
        &self,
        r: &Resolved,
        path: Option<String>,
        checkout: &ReviewCheckout,
    ) -> Result<ado_core::git::ReviewCheckoutResult> {
        let info = PullRequestGitInfo::from_json(&r.pull_request)?;
        if info.id != r.locator.id
            || !info
                .target
                .organization
                .eq_ignore_ascii_case(&r.locator.organization.name)
        {
            return Err(Error(
                "Unsupported, mismatched, or unsafe Azure Git remote.".into(),
            ));
        }
        checkout.prepare(&info, path.map(standardize_path).transpose()?)
    }
    pub(super) fn edit_comment(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
        thread_id: i64,
        comment_id: i64,
        body_file: &str,
        expected_body_file: &str,
    ) -> Result<()> {
        let r = self.resolve(target, profile_name)?;
        let repo = repository_identity(&r.pull_request)?;
        let body = read_body(body_file)?;
        let expected = read_body(expected_body_file)?;
        let posting = r.client.identity()?;
        if posting.id != r.profile.identity.id {
            return Err(Error(
                "The saved token authenticated as a different account. No comment was edited."
                    .into(),
            ));
        }
        let threads = r
            .client
            .threads(&repo.project, &repo.repository, r.locator.id)?;
        validate_edit_target(&threads, thread_id, comment_id, &posting.id, &expected)?;
        let updated = r.client.update_comment(&repo.project, &repo.repository, r.locator.id, thread_id, comment_id, &body)
            .map_err(|e| Error(format!("Comment edit was not confirmed: {e}. Inspect thread {thread_id} before retrying; the request may have reached Azure DevOps.")))?;
        let verify = (|| {
            let threads = r
                .client
                .threads(&repo.project, &repo.repository, r.locator.id)?;
            validate_edit_target(&threads, thread_id, comment_id, &posting.id, &body)
        })();
        if let Err(e) = verify {
            return Err(Error(format!(
                "Comment {comment_id} in thread {thread_id} was edited, but verification failed: {e}. Inspect that thread before retrying."
            )));
        }
        write_json(&updated)
    }

    pub(super) fn top_level_comment(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
        body_file: &str,
        commit: &str,
    ) -> Result<()> {
        let r = self.resolve(target, profile_name)?;
        let repo = repository_identity(&r.pull_request)?;
        let body = read_body(body_file)?;
        let posting = r.client.identity()?;
        if posting.id != r.profile.identity.id {
            return Err(Error(format!(
                "The saved token authenticated as a different account. No comment was posted; run 'ado auth update {}'.",
                r.profile.name
            )));
        }
        verify_source_commit(&r.pull_request, commit)?;
        let ready = r.client.pull_request(r.locator.id)?;
        verify_source_commit(&ready, commit)?;
        let created = r.client.create_top_level_thread(&repo.project, &repo.repository, r.locator.id, &body)
            .map_err(|e| Error(format!("Comment submission was not confirmed: {e} Inspect PR threads before retrying; the request may have reached Azure DevOps.")))?;
        let Some(thread_id) = created.get("id").and_then(integer).filter(|id| *id > 0) else {
            return Err(Error("The top-level comment was created, but Azure DevOps returned no thread ID. Do not retry automatically; inspect the pull request threads first.".into()));
        };
        let verify = (|| {
            validate_top_level_thread(&created, thread_id, &body, &posting.id)?;
            let threads = r
                .client
                .threads(&repo.project, &repo.repository, r.locator.id)?;
            let thread = threads
                .iter()
                .find(|thread| thread.get("id").and_then(integer) == Some(thread_id))
                .ok_or_else(|| {
                    Error("the new thread was absent from the immediate readback".into())
                })?;
            validate_top_level_thread(thread, thread_id, &body, &posting.id)
        })();
        if let Err(e) = verify {
            return Err(Error(format!(
                "Top-level comment thread {thread_id} was created, but verification failed: {e}. Do not retry automatically; inspect that thread first."
            )));
        }
        write_json(&created)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn comment(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
        file: &str,
        line: i64,
        end_line: i64,
        side: &str,
        body_file: &str,
        commit: &str,
        iteration: i64,
        change_id: i64,
    ) -> Result<()> {
        let r = self.resolve(target, profile_name)?;
        let repo = repository_identity(&r.pull_request)?;
        let path = comment_path(file)?;
        let body = read_body(body_file)?;
        let posting = r.client.identity()?;
        if posting.id != r.profile.identity.id {
            return Err(Error(format!(
                "The saved token authenticated as a different account. No comment was posted; run 'ado auth update {}'.",
                r.profile.name
            )));
        }
        verify_source_commit(&r.pull_request, commit)?;
        let before = r
            .client
            .iterations(&repo.project, &repo.repository, r.locator.id)?;
        if latest_iteration(&before)? != iteration {
            return Err(Error(
                "The requested iteration is no longer current; refresh changes and retry.".into(),
            ));
        }
        let changes = r
            .client
            .changes(&repo.project, &repo.repository, r.locator.id, iteration)?;
        let selected = changes
            .iter()
            .find(|c| {
                c.get("changeTrackingId").and_then(integer) == Some(change_id)
                    && change_path(c, side).as_deref() == Some(&path)
            })
            .ok_or_else(|| {
                Error(
                    "The change ID does not identify the requested file in this iteration.".into(),
                )
            })?;
        comment_anchor::validate_side(side, selected.get("changeType"))?;
        let ready_pr = r.client.pull_request(r.locator.id)?;
        verify_source_commit(&ready_pr, commit)?;
        let ready_iterations =
            r.client
                .iterations(&repo.project, &repo.repository, r.locator.id)?;
        if latest_iteration(&ready_iterations)? != iteration {
            return Err(Error("The requested iteration changed while preparing the comment; refresh changes and retry.".into()));
        }
        let anchor = iteration_commits(&ready_iterations, iteration, commit, side == "left")?;
        let anchor_repo = anchor_repository(side, &ready_pr, &repo, &r.locator.organization)?;
        let anchor_path = change_path(selected, side).ok_or_else(|| {
            Error("Azure DevOps did not return a path for the requested comment side.".into())
        })?;
        let anchor_commit = if side == "right" {
            &anchor.source
        } else {
            anchor.common.as_ref().unwrap()
        };
        let content =
            r.client
                .file_content(&anchor_repo.0, &anchor_repo.1, &anchor_path, anchor_commit)?;
        comment_anchor::validate(side, line, end_line, &content, selected.get("changeType"))?;
        let final_pr = r.client.pull_request(r.locator.id)?;
        verify_source_commit(&final_pr, commit)?;
        let final_iterations =
            r.client
                .iterations(&repo.project, &repo.repository, r.locator.id)?;
        if latest_iteration(&final_iterations)? != iteration
            || iteration_commits(&final_iterations, iteration, commit, side == "left")? != anchor
        {
            return Err(Error("The reviewed commits changed while validating the comment anchor; refresh changes and retry.".into()));
        }
        let created=r.client.create_thread(&repo.project,&repo.repository,r.locator.id,&body,&path,line,end_line,side,iteration,change_id).map_err(|e|Error(format!("Comment submission was not confirmed: {e} Inspect PR threads before retrying; the request may have reached Azure DevOps.")))?;
        let thread_id = created.get("id").and_then(integer).filter(|x| *x > 0);
        let Some(thread_id) = thread_id else {
            return Err(Error("The inline comment was created, but Azure DevOps returned no thread ID. Do not retry automatically; inspect the pull request threads first.".into()));
        };
        let verify = (|| {
            validate_thread(&created, thread_id, &path, line, end_line, side)?;
            let after = r.client.pull_request(r.locator.id)?;
            verify_source_commit(&after, commit)?;
            let iterations = r
                .client
                .iterations(&repo.project, &repo.repository, r.locator.id)?;
            if latest_iteration(&iterations)? != iteration {
                return Err(Error(
                    "the pull request source or iteration changed during creation".into(),
                ));
            }
            let threads = r
                .client
                .threads(&repo.project, &repo.repository, r.locator.id)?;
            let thread = threads
                .iter()
                .find(|t| t.get("id").and_then(integer) == Some(thread_id))
                .ok_or_else(|| {
                    Error("the new thread was absent from the immediate readback".into())
                })?;
            validate_thread(thread, thread_id, &path, line, end_line, side)
        })();
        if let Err(e) = verify {
            return Err(Error(format!(
                "Inline comment thread {thread_id} was created, but verification failed: {e}. Do not retry automatically; inspect that thread first."
            )));
        }
        write_json(&created)
    }
}

fn repository_identity(v: &Value) -> Result<RepositoryIdentity> {
    let repo = v.get("repository").ok_or_else(|| {
        Error("Azure DevOps pull request response is missing repository project and name.".into())
    })?;
    let repository = repo
        .get("name")
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .ok_or_else(|| {
            Error(
                "Azure DevOps pull request response is missing repository project and name.".into(),
            )
        })?;
    let project = repo
        .pointer("/project/name")
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .ok_or_else(|| {
            Error(
                "Azure DevOps pull request response is missing repository project and name.".into(),
            )
        })?;
    Ok(RepositoryIdentity {
        project: project.into(),
        repository: repository.into(),
        project_aliases: [
            Some(project),
            repo.pointer("/project/id").and_then(Value::as_str),
        ]
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .collect(),
        repository_aliases: [Some(repository), repo.get("id").and_then(Value::as_str)]
            .into_iter()
            .flatten()
            .map(str::to_owned)
            .collect(),
    })
}
fn requested_iteration(requested: Option<i64>, iterations: &[Value]) -> Result<i64> {
    match requested {
        Some(value) => Ok(value),
        None => latest_iteration(iterations),
    }
}
fn latest_iteration(v: &[Value]) -> Result<i64> {
    v.iter()
        .filter_map(|x| x.get("id").and_then(integer))
        .max()
        .filter(|x| *x > 0)
        .ok_or_else(|| {
            Error("Azure DevOps pull request response is missing pull request iteration.".into())
        })
}
fn verify_source_commit(v: &Value, expected: &str) -> Result<()> {
    let actual = v
        .pointer("/lastMergeSourceCommit/commitId")
        .and_then(Value::as_str)
        .filter(|s| is_commit_sha(s))
        .ok_or_else(|| {
            Error("Azure DevOps pull request response is missing reviewed source commit.".into())
        })?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(Error(
            "The pull request source commit does not match --commit; refresh the review and retry."
                .into(),
        ))
    }
}
#[derive(Debug, PartialEq, Eq)]
struct IterationCommits {
    source: String,
    common: Option<String>,
}
fn iteration_commits(
    values: &[Value],
    id: i64,
    expected: &str,
    need_common: bool,
) -> Result<IterationCommits> {
    let v = values
        .iter()
        .find(|v| v.get("id").and_then(integer) == Some(id))
        .ok_or_else(|| {
            Error("Azure DevOps pull request response is missing iteration sourceRefCommit.".into())
        })?;
    let source = v
        .pointer("/sourceRefCommit/commitId")
        .and_then(Value::as_str)
        .filter(|s| is_commit_sha(s))
        .ok_or_else(|| {
            Error("Azure DevOps pull request response is missing iteration sourceRefCommit.".into())
        })?;
    if !source.eq_ignore_ascii_case(expected) {
        return Err(Error(
            "The iteration source commit does not match --commit; refresh changes and retry."
                .into(),
        ));
    }
    let common = if need_common {
        Some(
            v.pointer("/commonRefCommit/commitId")
                .and_then(Value::as_str)
                .filter(|s| is_commit_sha(s))
                .ok_or_else(|| {
                    Error(
                        "Azure DevOps pull request response is missing iteration commonRefCommit."
                            .into(),
                    )
                })?
                .to_ascii_lowercase(),
        )
    } else {
        None
    };
    Ok(IterationCommits {
        source: source.to_ascii_lowercase(),
        common,
    })
}
fn anchor_repository(
    side: &str,
    pr: &Value,
    target: &RepositoryIdentity,
    org: &Organization,
) -> Result<(String, String)> {
    if side != "right" || pr.get("forkSource").is_none_or(Value::is_null) {
        return Ok((target.project.clone(), target.repository.clone()));
    }
    let repo = pr.pointer("/forkSource/repository").ok_or_else(|| {
        Error("The fork source repository metadata is unavailable; no comment was posted.".into())
    })?;
    let ssh = repo.get("sshUrl").and_then(Value::as_str).ok_or_else(|| {
        Error("The fork source repository metadata is unavailable; no comment was posted.".into())
    })?;
    let remote=AzureGitRemote::validated_ssh(ssh,None).map_err(|_|Error("The fork source repository SSH metadata is not a trusted Azure URL; no comment was posted.".into()))?;
    if !remote.organization.eq_ignore_ascii_case(&org.name) {
        return Err(Error(
            "Cross-organization fork comments are not supported.".into(),
        ));
    }
    let pa: [Option<&str>; 2] = [
        repo.pointer("/project/id").and_then(Value::as_str),
        repo.pointer("/project/name").and_then(Value::as_str),
    ];
    let ra: [Option<&str>; 2] = [
        repo.get("id").and_then(Value::as_str),
        repo.get("name").and_then(Value::as_str),
    ];
    if !pa
        .iter()
        .flatten()
        .any(|x| x.eq_ignore_ascii_case(&remote.project))
        || !ra
            .iter()
            .flatten()
            .any(|x| x.eq_ignore_ascii_case(&remote.repository))
    {
        return Err(Error("The fork source repository metadata does not match its Azure SSH URL; no comment was posted.".into()));
    }
    let p = pa[0].or(pa[1]).ok_or_else(|| {
        Error("The fork source repository metadata is unavailable; no comment was posted.".into())
    })?;
    let r = ra[0].or(ra[1]).ok_or_else(|| {
        Error("The fork source repository metadata is unavailable; no comment was posted.".into())
    })?;
    Ok((p.into(), r.into()))
}
fn comment_path(raw: &str) -> Result<String> {
    let p = if raw.starts_with('/') {
        raw.into()
    } else {
        format!("/{raw}")
    };
    if p.len() <= 1 || p.contains(['\0', '\n', '\r']) {
        Err(Error("--file must be a nonempty repository path.".into()))
    } else {
        Ok(p)
    }
}
fn change_path(v: &Value, side: &str) -> Option<String> {
    let p = if side == "left" {
        v.get("originalPath").and_then(Value::as_str)
    } else {
        v.pointer("/item/path").and_then(Value::as_str)
    }?;
    Some(if p.starts_with('/') {
        p.into()
    } else {
        format!("/{p}")
    })
}
fn validate_edit_target(
    threads: &[Value],
    thread_id: i64,
    comment_id: i64,
    author: &str,
    expected: &str,
) -> Result<()> {
    let comment = threads
        .iter()
        .find(|t| {
            t.get("id").and_then(integer) == Some(thread_id)
                && t.get("isDeleted").and_then(Value::as_bool) != Some(true)
        })
        .and_then(|t| t.get("comments").and_then(Value::as_array))
        .and_then(|cs| {
            cs.iter()
                .find(|c| c.get("id").and_then(integer) == Some(comment_id))
        })
        .ok_or_else(|| Error("The requested comment was not found.".into()))?;
    if comment.pointer("/author/id").and_then(Value::as_str) != Some(author)
        || comment.get("isDeleted").and_then(Value::as_bool) == Some(true)
    {
        return Err(Error(
            "Only the authenticated user's own, undeleted comment may be edited.".into(),
        ));
    }
    // Tolerate the single trailing newline that `echo` or `jq -r` add to the file.
    let trimmed = expected
        .strip_suffix("\r\n")
        .or_else(|| expected.strip_suffix('\n'))
        .unwrap_or(expected);
    let content = comment.get("content").and_then(Value::as_str);
    if content != Some(expected) && content != Some(trimmed) {
        return Err(Error(
            "The comment text changed; reread it and refresh --expected-body-file before editing."
                .into(),
        ));
    }
    Ok(())
}

fn validate_top_level_thread(v: &Value, id: i64, body: &str, author: &str) -> Result<()> {
    let comments = v.get("comments").and_then(Value::as_array);
    if v.get("id").and_then(integer) == Some(id)
        && v.get("threadContext").is_none_or(Value::is_null)
        && v.get("pullRequestThreadContext").is_none_or(Value::is_null)
        && v.get("isDeleted").and_then(Value::as_bool) != Some(true)
        && comments.is_some_and(|comments| {
            comments.iter().any(|comment| {
                comment.get("content").and_then(Value::as_str) == Some(body)
                    && comment.pointer("/author/id").and_then(Value::as_str) == Some(author)
                    && comment.get("parentCommentId").and_then(integer) == Some(0)
                    && comment.get("isDeleted").and_then(Value::as_bool) != Some(true)
            })
        })
    {
        Ok(())
    } else {
        Err(Error(
            "thread readback did not match the requested top-level comment, body and author".into(),
        ))
    }
}

fn validate_thread(v: &Value, id: i64, path: &str, line: i64, end: i64, side: &str) -> Result<()> {
    if v.get("id").and_then(integer) == Some(id)
        && v.pointer("/threadContext/filePath").and_then(Value::as_str) == Some(path)
        && v.pointer(&format!("/threadContext/{side}FileStart/line"))
            .and_then(integer)
            == Some(line)
        && v.pointer(&format!("/threadContext/{side}FileEnd/line"))
            .and_then(integer)
            == Some(end)
    {
        Ok(())
    } else {
        Err(Error(
            "thread readback did not match the requested file and line context".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::management::{parse_labels, pull_request_url, validate_source_tip};
    use super::*;

    #[test]
    fn top_level_readback_checks_content_author_and_placement() {
        let thread = json!({"id": 7, "comments": [{"content": "Scope?", "author": {"id": "ollie"}, "parentCommentId": 0}]});
        assert!(validate_top_level_thread(&thread, 7, "Scope?", "ollie").is_ok());
        assert!(validate_top_level_thread(&thread, 8, "Scope?", "ollie").is_err());
        assert!(validate_top_level_thread(&thread, 7, "other", "ollie").is_err());
        assert!(validate_top_level_thread(&thread, 7, "Scope?", "other").is_err());
        for (pointer, value) in [
            ("/threadContext", json!({"filePath": "/file"})),
            ("/pullRequestThreadContext", json!({"changeTrackingId": 1})),
            ("/isDeleted", json!(true)),
        ] {
            let mut invalid = thread.clone();
            invalid[pointer.trim_start_matches('/')] = value;
            assert!(validate_top_level_thread(&invalid, 7, "Scope?", "ollie").is_err());
        }
        let mut reply = thread.clone();
        reply["comments"][0]["parentCommentId"] = json!(1);
        assert!(validate_top_level_thread(&reply, 7, "Scope?", "ollie").is_err());
        let mut deleted = thread.clone();
        deleted["comments"][0]["isDeleted"] = json!(true);
        assert!(validate_top_level_thread(&deleted, 7, "Scope?", "ollie").is_err());
        let mut null_context = thread;
        null_context["threadContext"] = Value::Null;
        assert!(validate_top_level_thread(&null_context, 7, "Scope?", "ollie").is_ok());
    }

    #[test]
    fn edits_require_own_undeleted_comment_and_expected_text() {
        let thread = json!({"id": 7, "comments": [{"id": 1, "content": "before", "author": {"id": "ollie"}}]});
        assert!(
            validate_edit_target(std::slice::from_ref(&thread), 7, 1, "ollie", "before").is_ok()
        );
        assert!(
            validate_edit_target(std::slice::from_ref(&thread), 7, 1, "other", "before").is_err()
        );
        assert!(
            validate_edit_target(std::slice::from_ref(&thread), 7, 1, "ollie", "stale").is_err()
        );
        for expected in ["before\n", "before\r\n"] {
            assert!(
                validate_edit_target(std::slice::from_ref(&thread), 7, 1, "ollie", expected)
                    .is_ok()
            );
        }
        assert!(
            validate_edit_target(std::slice::from_ref(&thread), 7, 1, "ollie", "before\n\n")
                .is_err()
        );
        assert!(
            validate_edit_target(std::slice::from_ref(&thread), 8, 1, "ollie", "before").is_err()
        );
        assert!(
            validate_edit_target(std::slice::from_ref(&thread), 7, 2, "ollie", "before").is_err()
        );
        let mut deleted = thread.clone();
        deleted["comments"][0]["isDeleted"] = json!(true);
        assert!(validate_edit_target(&[deleted], 7, 1, "ollie", "before").is_err());
        let mut deleted = thread;
        deleted["isDeleted"] = json!(true);
        assert!(validate_edit_target(&[deleted], 7, 1, "ollie", "before").is_err());
    }

    #[test]
    fn explicit_iteration_does_not_evaluate_latest_iteration() {
        assert_eq!(requested_iteration(Some(7), &[]).unwrap(), 7);
        assert_eq!(
            requested_iteration(None, &[]).unwrap_err().to_string(),
            "Azure DevOps pull request response is missing pull request iteration."
        );
    }

    #[test]
    fn create_requires_remote_source_and_current_head_match() {
        let head = "a".repeat(40);
        assert!(
            validate_source_tip(
                "topic",
                "refs/heads/topic",
                "refs/heads/topic",
                Some(&head),
                Some(&head),
            )
            .is_ok()
        );
        assert!(
            validate_source_tip(
                "topic",
                "refs/heads/topic",
                "refs/heads/topic",
                Some(&head),
                Some(&"b".repeat(40)),
            )
            .unwrap_err()
            .to_string()
            .contains("Push the current branch")
        );
        assert!(
            validate_source_tip(
                "other",
                "refs/heads/other",
                "refs/heads/topic",
                None,
                Some(&"b".repeat(40)),
            )
            .is_ok()
        );
        assert!(
            validate_source_tip(
                "missing",
                "refs/heads/missing",
                "refs/heads/topic",
                None,
                None,
            )
            .unwrap_err()
            .to_string()
            .contains("does not exist")
        );
    }

    #[test]
    fn pull_request_output_builds_encoded_web_url_and_comma_labels() {
        assert_eq!(
            pull_request_url(
                &Organization::parse("acme").unwrap(),
                "My Project",
                "Repo Name",
                42,
            )
            .unwrap(),
            "https://dev.azure.com/acme/My%20Project/_git/Repo%20Name/pullrequest/42"
        );
        assert_eq!(
            parse_labels(" urgent,API, api, ,needs review "),
            vec!["API", "needs review", "urgent"]
        );
    }
}
