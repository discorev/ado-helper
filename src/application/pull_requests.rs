use super::{
    Application,
    output::{integer, open_browser, read_body, standardize_path, write_json, write_value},
};
use ado_core::{
    Error, Result,
    ado_client::AdoClient,
    cli::{AutoCompleteOptions, PullRequestTarget},
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

mod checks;
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

    pub(super) fn pr_vote(
        &self,
        target: PullRequestTarget,
        vote: &str,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let r = self.resolve(target, profile_name)?;
        let repo = repository_identity(&r.pull_request)?;
        let identity = r.client.identity()?;
        if identity.id != r.profile.identity.id {
            return Err(Error(format!(
                "The saved token authenticated as a different account. No vote was cast; run 'ado auth update {}'.",
                r.profile.name
            )));
        }
        let is_required = reviewer_is_required(&r.pull_request, &identity.id);
        let updated = r.client.set_pull_request_vote(
            &repo.project,
            &repo.repository,
            r.locator.id,
            &identity.id,
            vote_value(vote),
            is_required,
        )?;
        write_json(&updated)
    }

    pub(super) fn pr_complete(
        &self,
        target: PullRequestTarget,
        merge_strategy: &str,
        delete_source_branch: bool,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let r = self.resolve(target, profile_name)?;
        let repo = repository_identity(&r.pull_request)?;
        let body = completion_body(&r.pull_request, merge_strategy, delete_source_branch)?;
        let updated =
            match r
                .client
                .update_pull_request(&repo.project, &repo.repository, r.locator.id, &body)
            {
                Ok(value) => value,
                Err(error) => {
                    // The PATCH may have failed because a blocking policy has not passed yet; look
                    // that up as a best-effort addition to the error rather than letting it mask the
                    // original failure.
                    let evaluations = checks::project_id(&r.pull_request).and_then(|project_id| {
                        r.client.policy_evaluations(&project_id, r.locator.id)
                    });
                    return Err(checks::completion_error_with_policies(
                        error,
                        evaluations,
                        r.locator.id,
                    ));
                }
            };
        write_json(&updated)
    }

    pub(super) fn pr_auto_complete(
        &self,
        target: PullRequestTarget,
        options: Option<&AutoCompleteOptions>,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let r = self.resolve(target, profile_name)?;
        let repo = repository_identity(&r.pull_request)?;
        let identity = r.client.identity()?;
        if identity.id != r.profile.identity.id {
            return Err(Error(format!(
                "The saved token authenticated as a different account. No auto-complete change was made; run 'ado auth update {}'.",
                r.profile.name
            )));
        }
        let verified = set_auto_complete(
            &r.client,
            &repo.project,
            &repo.repository,
            r.locator.id,
            &r.pull_request,
            &identity.id,
            options,
        )?;
        write_json(&verified)
    }

    pub(super) fn pr_thread_status(
        &self,
        target: PullRequestTarget,
        thread_id: i64,
        status: &str,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let r = self.resolve(target, profile_name)?;
        let repo = repository_identity(&r.pull_request)?;
        let updated = r.client.update_thread_status(
            &repo.project,
            &repo.repository,
            r.locator.id,
            thread_id,
            thread_status_value(status),
        )?;
        write_json(&updated)
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

    pub(super) fn reply_comment(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
        thread_id: i64,
        parent_comment_id: Option<i64>,
        body_file: &str,
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
        let threads = r
            .client
            .threads(&repo.project, &repo.repository, r.locator.id)?;
        let parent = reply_parent(&threads, thread_id, parent_comment_id)?;
        let created = r.client.create_comment(&repo.project, &repo.repository, r.locator.id, thread_id, parent, &body)
            .map_err(|e| Error(format!("Reply submission was not confirmed: {e} Inspect thread {thread_id} before retrying; the request may have reached Azure DevOps.")))?;
        let Some(comment_id) = created.get("id").and_then(integer).filter(|id| *id > 0) else {
            return Err(Error(format!(
                "The reply was created, but Azure DevOps returned no comment ID. Do not retry automatically; inspect thread {thread_id} first."
            )));
        };
        let verify = (|| {
            validate_reply(&created, comment_id, parent, &body, &posting.id)?;
            let threads = r
                .client
                .threads(&repo.project, &repo.repository, r.locator.id)?;
            let comment = threads
                .iter()
                .find(|thread| thread.get("id").and_then(integer) == Some(thread_id))
                .and_then(|thread| thread.get("comments").and_then(Value::as_array))
                .and_then(|comments| {
                    comments
                        .iter()
                        .find(|comment| comment.get("id").and_then(integer) == Some(comment_id))
                })
                .ok_or_else(|| {
                    Error("the new reply was absent from the immediate readback".into())
                })?;
            validate_reply(comment, comment_id, parent, &body, &posting.id)
        })();
        if let Err(e) = verify {
            return Err(Error(format!(
                "Reply {comment_id} in thread {thread_id} was created, but verification failed: {e}. Do not retry automatically; inspect that thread first."
            )));
        }
        write_json(&created)
    }

    pub(super) fn delete_comment(
        &self,
        target: PullRequestTarget,
        profile_name: Option<&str>,
        thread_id: i64,
        comment_id: i64,
        expected_body_file: &str,
    ) -> Result<()> {
        let r = self.resolve(target, profile_name)?;
        let repo = repository_identity(&r.pull_request)?;
        let expected = read_body(expected_body_file)?;
        let posting = r.client.identity()?;
        if posting.id != r.profile.identity.id {
            return Err(Error(
                "The saved token authenticated as a different account. No comment was deleted."
                    .into(),
            ));
        }
        let threads = r
            .client
            .threads(&repo.project, &repo.repository, r.locator.id)?;
        validate_edit_target(&threads, thread_id, comment_id, &posting.id, &expected)?;
        r.client
            .delete_comment(&repo.project, &repo.repository, r.locator.id, thread_id, comment_id)
            .map_err(|e| Error(format!("Comment deletion was not confirmed: {e} Inspect thread {thread_id} before retrying; the request may have reached Azure DevOps.")))?;
        let verify = (|| {
            let threads = r
                .client
                .threads(&repo.project, &repo.repository, r.locator.id)?;
            validate_deleted(&threads, thread_id, comment_id)
        })();
        if let Err(e) = verify {
            return Err(Error(format!(
                "Comment {comment_id} in thread {thread_id} was deleted, but verification failed: {e}. Inspect that thread before retrying."
            )));
        }
        write_json(
            &json!({"pullRequestId": r.locator.id, "threadId": thread_id, "commentId": comment_id, "deleted": true}),
        )
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

fn vote_value(vote: &str) -> i32 {
    match vote {
        "approve" => 10,
        "approve-with-suggestions" => 5,
        "wait" => -5,
        "reject" => -10,
        "reset" => 0,
        _ => unreachable!(),
    }
}
fn merge_strategy_value(strategy: &str) -> &'static str {
    match strategy {
        "no-fast-forward" => "noFastForward",
        "squash" => "squash",
        "rebase" => "rebase",
        "rebase-merge" => "rebaseMerge",
        _ => unreachable!(),
    }
}

fn completion_body(
    pull_request: &Value,
    merge_strategy: &str,
    delete_source_branch: bool,
) -> Result<Value> {
    if pull_request["isDraft"].as_bool() == Some(true) {
        return Err(Error(
            "The pull request is a draft. Mark it ready before completing it.".into(),
        ));
    }
    if pull_request["status"].as_str() != Some("active") {
        return Err(Error(
            "Only an active pull request can be completed.".into(),
        ));
    }
    let merge_status = pull_request["mergeStatus"].as_str().unwrap_or("notSet");
    if merge_status != "succeeded" {
        return Err(Error(format!(
            "The pull request merge status is '{merge_status}', not 'succeeded'. Nothing was completed."
        )));
    }
    let commit = pull_request["lastMergeSourceCommit"]["commitId"]
        .as_str()
        .filter(|commit| !commit.is_empty())
        .ok_or_else(|| Error("The pull request has no source commit to complete.".into()))?;
    Ok(json!({
        "status": "completed",
        "lastMergeSourceCommit": {"commitId": commit},
        "completionOptions": {
            "mergeStrategy": merge_strategy_value(merge_strategy),
            "deleteSourceBranch": delete_source_branch,
        },
    }))
}

const CLEARED_IDENTITY: &str = "00000000-0000-0000-0000-000000000000";

/// Builds the PATCH body to set or clear auto-complete. When setting, the PR's existing
/// `completionOptions` (e.g. `transitionWorkItems`, `mergeCommitMessage`) are sent back with only
/// `mergeStrategy` and `deleteSourceBranch` overwritten, so a PATCH that replaces the object
/// cannot silently reset them.
fn auto_complete_body(
    pull_request: &Value,
    identity_id: &str,
    options: Option<&AutoCompleteOptions>,
) -> Value {
    match options {
        Some(options) => {
            let mut completion_options = pull_request
                .get("completionOptions")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            completion_options.insert(
                "mergeStrategy".into(),
                json!(options.merge_strategy.as_str()),
            );
            completion_options.insert(
                "deleteSourceBranch".into(),
                json!(options.delete_source_branch),
            );
            json!({
                "autoCompleteSetBy": {"id": identity_id},
                "completionOptions": completion_options,
            })
        }
        None => json!({"autoCompleteSetBy": {"id": CLEARED_IDENTITY}}),
    }
}

fn validate_auto_complete(
    pull_request: &Value,
    identity_id: &str,
    options: Option<&AutoCompleteOptions>,
) -> Result<()> {
    let set_by_id = pull_request
        .pointer("/autoCompleteSetBy/id")
        .and_then(Value::as_str);
    match options {
        Some(options) => {
            if !set_by_id.is_some_and(|id| id.eq_ignore_ascii_case(identity_id)) {
                return Err(Error(format!(
                    "autoCompleteSetBy.id was '{}', not the authenticated user.",
                    set_by_id.unwrap_or("<absent>")
                )));
            }
            let actual_strategy = pull_request
                .pointer("/completionOptions/mergeStrategy")
                .and_then(Value::as_str);
            if actual_strategy != Some(options.merge_strategy.as_str()) {
                return Err(Error(format!(
                    "completionOptions.mergeStrategy was '{}', not '{}'.",
                    actual_strategy.unwrap_or("<absent>"),
                    options.merge_strategy
                )));
            }
            // Azure DevOps may omit a false boolean from the response, so absent means false.
            let actual_delete = pull_request
                .pointer("/completionOptions/deleteSourceBranch")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if actual_delete != options.delete_source_branch {
                return Err(Error(format!(
                    "completionOptions.deleteSourceBranch was '{actual_delete}', not '{}'.",
                    options.delete_source_branch
                )));
            }
            Ok(())
        }
        None => {
            let cleared = match set_by_id {
                None => true,
                Some(id) => id.eq_ignore_ascii_case(CLEARED_IDENTITY),
            };
            if cleared {
                Ok(())
            } else {
                Err(Error(format!(
                    "autoCompleteSetBy.id was still '{}', not cleared.",
                    set_by_id.unwrap_or("<absent>")
                )))
            }
        }
    }
}

/// Patches auto-complete, reads the pull request back, and verifies the change. Shared by
/// `pr auto-complete` and `pr create --auto-complete` so the latter can reuse the same
/// patch-then-verify behaviour against its own final readback.
#[allow(clippy::too_many_arguments)]
fn set_auto_complete(
    client: &AdoClient,
    project: &str,
    repository: &str,
    id: i64,
    current_pull_request: &Value,
    identity_id: &str,
    options: Option<&AutoCompleteOptions>,
) -> Result<Value> {
    let body = auto_complete_body(current_pull_request, identity_id, options);
    // A failed PATCH may still have been applied (for example a timeout), so every failure is
    // reported as unconfirmed rather than as unchanged.
    client
        .update_pull_request(project, repository, id, &body)
        .and_then(|_| client.pull_request(id))
        .and_then(|readback| validate_auto_complete(&readback, identity_id, options).map(|()| readback))
        .map_err(|Error(reason)| {
            Error(format!(
                "Auto-complete was not confirmed: {}. The request may have reached Azure DevOps; check with 'ado pr show' before relying on it.",
                reason.trim_end_matches('.')
            ))
        })
}

fn thread_status_value(status: &str) -> &'static str {
    match status {
        "active" => "active",
        "fixed" => "fixed",
        "wontfix" => "wontFix",
        "closed" => "closed",
        "bydesign" => "byDesign",
        "pending" => "pending",
        _ => unreachable!(),
    }
}
fn reviewer_is_required(pull_request: &Value, identity_id: &str) -> Option<bool> {
    pull_request
        .get("reviewers")
        .and_then(Value::as_array)?
        .iter()
        .find(|reviewer| reviewer.get("id").and_then(Value::as_str) == Some(identity_id))
        .map(|reviewer| {
            reviewer
                .get("isRequired")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
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
            "Only the authenticated user's own, undeleted comment may be edited or deleted.".into(),
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
            "The comment text changed; reread it and refresh --expected-body-file before retrying."
                .into(),
        ));
    }
    Ok(())
}

fn reply_parent(threads: &[Value], thread_id: i64, requested: Option<i64>) -> Result<i64> {
    let thread = threads
        .iter()
        .find(|t| {
            t.get("id").and_then(integer) == Some(thread_id)
                && t.get("isDeleted").and_then(Value::as_bool) != Some(true)
        })
        .ok_or_else(|| Error(format!("Thread {thread_id} was not found.")))?;
    let comments: &[Value] = thread
        .get("comments")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    match requested {
        Some(parent) => {
            let found = comments.iter().any(|comment| {
                comment.get("id").and_then(integer) == Some(parent)
                    && comment.get("isDeleted").and_then(Value::as_bool) != Some(true)
            });
            if found {
                Ok(parent)
            } else {
                Err(Error(format!(
                    "Comment {parent} was not found in thread {thread_id}."
                )))
            }
        }
        None => comments
            .iter()
            .filter(|comment| comment.get("isDeleted").and_then(Value::as_bool) != Some(true))
            .filter_map(|comment| comment.get("id").and_then(integer))
            .min()
            .ok_or_else(|| {
                Error(format!(
                    "Thread {thread_id} has no undeleted comment to reply to."
                ))
            }),
    }
}

fn validate_reply(comment: &Value, id: i64, parent: i64, body: &str, author: &str) -> Result<()> {
    if comment.get("id").and_then(integer) == Some(id)
        && comment.get("content").and_then(Value::as_str) == Some(body)
        && comment.pointer("/author/id").and_then(Value::as_str) == Some(author)
        && comment.get("parentCommentId").and_then(integer) == Some(parent)
        && comment.get("isDeleted").and_then(Value::as_bool) != Some(true)
    {
        Ok(())
    } else {
        Err(Error(
            "comment readback did not match the requested reply, parent, body and author".into(),
        ))
    }
}

fn validate_deleted(threads: &[Value], thread_id: i64, comment_id: i64) -> Result<()> {
    let still_present = threads
        .iter()
        .find(|t| t.get("id").and_then(integer) == Some(thread_id))
        .is_some_and(|thread| {
            thread.get("isDeleted").and_then(Value::as_bool) != Some(true)
                && thread
                    .get("comments")
                    .and_then(Value::as_array)
                    .is_some_and(|comments| {
                        comments.iter().any(|comment| {
                            comment.get("id").and_then(integer) == Some(comment_id)
                                && comment.get("isDeleted").and_then(Value::as_bool) != Some(true)
                        })
                    })
        });
    if still_present {
        Err(Error(
            "the comment is still present in the immediate readback".into(),
        ))
    } else {
        Ok(())
    }
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
    use super::management::{
        parse_labels, pull_request_url, validate_required_reviewers, validate_reviewers_present,
        validate_source_tip,
    };
    use super::*;

    #[test]
    fn completion_body_maps_strategy_and_pins_source_commit() {
        let pr = json!({"status": "active", "isDraft": false, "mergeStatus": "succeeded", "lastMergeSourceCommit": {"commitId": "abc"}});
        for (cli, ado) in [
            ("no-fast-forward", "noFastForward"),
            ("squash", "squash"),
            ("rebase", "rebase"),
            ("rebase-merge", "rebaseMerge"),
        ] {
            assert_eq!(
                completion_body(&pr, cli, true).unwrap(),
                json!({"status": "completed", "lastMergeSourceCommit": {"commitId": "abc"}, "completionOptions": {"mergeStrategy": ado, "deleteSourceBranch": true}})
            );
        }
    }

    #[test]
    fn completion_body_refuses_draft_inactive_and_unmerged() {
        let ok = json!({"status": "active", "isDraft": false, "mergeStatus": "succeeded", "lastMergeSourceCommit": {"commitId": "abc"}});
        assert!(completion_body(&ok, "squash", false).is_ok());
        for (field, value) in [
            ("isDraft", json!(true)),
            ("status", json!("abandoned")),
            ("mergeStatus", json!("conflicts")),
            ("mergeStatus", json!("queued")),
            ("mergeStatus", Value::Null),
            ("lastMergeSourceCommit", Value::Null),
        ] {
            let mut pr = ok.clone();
            pr[field] = value;
            assert!(completion_body(&pr, "squash", false).is_err(), "{field}");
        }
    }

    #[test]
    fn auto_complete_body_sets_identity_and_merge_options_without_existing_completion_options() {
        let pr = json!({});
        let options = AutoCompleteOptions {
            merge_strategy: "noFastForward".into(),
            delete_source_branch: true,
        };
        assert_eq!(
            auto_complete_body(&pr, "id-1", Some(&options)),
            json!({
                "autoCompleteSetBy": {"id": "id-1"},
                "completionOptions": {"mergeStrategy": "noFastForward", "deleteSourceBranch": true},
            })
        );
    }

    #[test]
    fn auto_complete_body_preserves_other_existing_completion_options() {
        let pr = json!({
            "completionOptions": {
                "mergeStrategy": "squash",
                "deleteSourceBranch": false,
                "transitionWorkItems": true,
                "mergeCommitMessage": "Merging",
            }
        });
        let options = AutoCompleteOptions {
            merge_strategy: "rebase".into(),
            delete_source_branch: true,
        };
        assert_eq!(
            auto_complete_body(&pr, "id-1", Some(&options)),
            json!({
                "autoCompleteSetBy": {"id": "id-1"},
                "completionOptions": {
                    "mergeStrategy": "rebase",
                    "deleteSourceBranch": true,
                    "transitionWorkItems": true,
                    "mergeCommitMessage": "Merging",
                },
            })
        );
    }

    #[test]
    fn auto_complete_body_clear_sends_only_the_zero_guid() {
        assert_eq!(
            auto_complete_body(&json!({}), "id-1", None),
            json!({"autoCompleteSetBy": {"id": CLEARED_IDENTITY}})
        );
        assert_eq!(CLEARED_IDENTITY, "00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn validate_auto_complete_set_checks_identity_strategy_and_delete_flag() {
        let options = AutoCompleteOptions {
            merge_strategy: "squash".into(),
            delete_source_branch: true,
        };
        let good = json!({
            "autoCompleteSetBy": {"id": "ID-1"},
            "completionOptions": {"mergeStrategy": "squash", "deleteSourceBranch": true},
        });
        assert!(validate_auto_complete(&good, "id-1", Some(&options)).is_ok());
        let wrong_id = json!({
            "autoCompleteSetBy": {"id": "other"},
            "completionOptions": {"mergeStrategy": "squash", "deleteSourceBranch": true},
        });
        assert!(validate_auto_complete(&wrong_id, "id-1", Some(&options)).is_err());
        let wrong_strategy = json!({
            "autoCompleteSetBy": {"id": "id-1"},
            "completionOptions": {"mergeStrategy": "rebase", "deleteSourceBranch": true},
        });
        assert!(validate_auto_complete(&wrong_strategy, "id-1", Some(&options)).is_err());
        let wrong_delete = json!({
            "autoCompleteSetBy": {"id": "id-1"},
            "completionOptions": {"mergeStrategy": "squash", "deleteSourceBranch": false},
        });
        assert!(validate_auto_complete(&wrong_delete, "id-1", Some(&options)).is_err());
        let keep = AutoCompleteOptions {
            delete_source_branch: false,
            ..options.clone()
        };
        let omitted_false = json!({
            "autoCompleteSetBy": {"id": "id-1"},
            "completionOptions": {"mergeStrategy": "squash"},
        });
        assert!(validate_auto_complete(&omitted_false, "id-1", Some(&keep)).is_ok());
        assert!(validate_auto_complete(&omitted_false, "id-1", Some(&options)).is_err());
        let missing_set_by = json!({
            "completionOptions": {"mergeStrategy": "squash", "deleteSourceBranch": true},
        });
        assert!(validate_auto_complete(&missing_set_by, "id-1", Some(&options)).is_err());
    }

    #[test]
    fn validate_auto_complete_clear_accepts_absent_null_or_zero_and_rejects_a_real_id() {
        assert!(validate_auto_complete(&json!({}), "id-1", None).is_ok());
        assert!(
            validate_auto_complete(&json!({"autoCompleteSetBy": Value::Null}), "id-1", None)
                .is_ok()
        );
        assert!(
            validate_auto_complete(
                &json!({"autoCompleteSetBy": {"id": CLEARED_IDENTITY}}),
                "id-1",
                None
            )
            .is_ok()
        );
        assert!(
            validate_auto_complete(&json!({"autoCompleteSetBy": {"id": "id-1"}}), "id-1", None)
                .is_err()
        );
    }

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
    fn reply_parent_defaults_to_lowest_undeleted_comment_id_regardless_of_order() {
        let thread = json!({"id": 7, "comments": [
            {"id": 3},
            {"id": 1},
            {"id": 2},
        ]});
        assert_eq!(
            reply_parent(std::slice::from_ref(&thread), 7, None).unwrap(),
            1
        );
        let mut without_one = thread.clone();
        without_one["comments"][1]["isDeleted"] = json!(true);
        assert_eq!(reply_parent(&[without_one], 7, None).unwrap(), 2);
    }

    #[test]
    fn reply_parent_honours_explicit_parent_and_rejects_missing_or_deleted() {
        let thread = json!({"id": 7, "comments": [
            {"id": 1},
            {"id": 2, "isDeleted": true},
        ]});
        assert_eq!(
            reply_parent(std::slice::from_ref(&thread), 7, Some(1)).unwrap(),
            1
        );
        assert!(reply_parent(std::slice::from_ref(&thread), 7, Some(2)).is_err());
        assert!(reply_parent(std::slice::from_ref(&thread), 7, Some(99)).is_err());
        assert!(reply_parent(&[], 7, Some(1)).is_err());
        assert!(reply_parent(&[], 7, None).is_err());
        let mut deleted_thread = thread;
        deleted_thread["isDeleted"] = json!(true);
        assert!(reply_parent(&[deleted_thread.clone()], 7, Some(1)).is_err());
        assert!(reply_parent(&[deleted_thread], 7, None).is_err());
    }

    #[test]
    fn validate_reply_checks_id_content_author_parent_and_deletion() {
        let comment =
            json!({"id": 5, "content": "On it", "author": {"id": "ollie"}, "parentCommentId": 1});
        assert!(validate_reply(&comment, 5, 1, "On it", "ollie").is_ok());
        assert!(validate_reply(&comment, 6, 1, "On it", "ollie").is_err());
        assert!(validate_reply(&comment, 5, 2, "On it", "ollie").is_err());
        assert!(validate_reply(&comment, 5, 1, "other", "ollie").is_err());
        assert!(validate_reply(&comment, 5, 1, "On it", "other").is_err());
        let mut deleted = comment;
        deleted["isDeleted"] = json!(true);
        assert!(validate_reply(&deleted, 5, 1, "On it", "ollie").is_err());
    }

    #[test]
    fn validate_deleted_ok_when_comment_or_thread_is_gone() {
        assert!(validate_deleted(&[], 7, 1).is_ok());
        let deleted_thread = json!({"id": 7, "isDeleted": true, "comments": [{"id": 1}]});
        assert!(validate_deleted(std::slice::from_ref(&deleted_thread), 7, 1).is_ok());
        let comment_absent = json!({"id": 7, "comments": [{"id": 2}]});
        assert!(validate_deleted(std::slice::from_ref(&comment_absent), 7, 1).is_ok());
        let comment_deleted = json!({"id": 7, "comments": [{"id": 1, "isDeleted": true}]});
        assert!(validate_deleted(std::slice::from_ref(&comment_deleted), 7, 1).is_ok());
        let still_present = json!({"id": 7, "comments": [{"id": 1}]});
        assert!(validate_deleted(&[still_present], 7, 1).is_err());
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

    #[test]
    fn vote_strings_map_to_azure_devops_vote_values() {
        assert_eq!(vote_value("approve"), 10);
        assert_eq!(vote_value("approve-with-suggestions"), 5);
        assert_eq!(vote_value("reset"), 0);
        assert_eq!(vote_value("wait"), -5);
        assert_eq!(vote_value("reject"), -10);
    }

    #[test]
    fn thread_status_strings_map_to_azure_devops_enum_casing() {
        assert_eq!(thread_status_value("active"), "active");
        assert_eq!(thread_status_value("fixed"), "fixed");
        assert_eq!(thread_status_value("wontfix"), "wontFix");
        assert_eq!(thread_status_value("closed"), "closed");
        assert_eq!(thread_status_value("bydesign"), "byDesign");
        assert_eq!(thread_status_value("pending"), "pending");
    }

    #[test]
    fn reviewer_is_required_reads_the_matching_reviewer_entry() {
        let pull_request = json!({
            "reviewers": [
                {"id": "ollie", "isRequired": true},
                {"id": "ada"},
            ]
        });
        assert_eq!(reviewer_is_required(&pull_request, "ollie"), Some(true));
        assert_eq!(reviewer_is_required(&pull_request, "ada"), Some(false));
        assert_eq!(reviewer_is_required(&pull_request, "grace"), None);
        assert_eq!(reviewer_is_required(&json!({}), "ollie"), None);
    }

    #[test]
    fn validate_required_reviewers_checks_presence_required_flag_and_vote() {
        let pull_request = json!({
            "pullRequestId": 42,
            "reviewers": [
                {"id": "OLLIE", "isRequired": true, "vote": 5},
                {"id": "ada", "vote": 0},
            ]
        });
        assert!(
            validate_required_reviewers(&pull_request, &[("ollie".into(), "ollie".into(), 5)])
                .is_ok()
        );
        assert_eq!(
            validate_required_reviewers(&pull_request, &[("grace".into(), "grace".into(), 0)])
                .unwrap_err()
                .to_string(),
            "Pull request 42 was updated, but reviewer 'grace' is not shown as required in the readback."
        );
        assert_eq!(
            validate_required_reviewers(&pull_request, &[("ada".into(), "ada".into(), 0)])
                .unwrap_err()
                .to_string(),
            "Pull request 42 was updated, but reviewer 'ada' is not shown as required in the readback."
        );
        assert_eq!(
            validate_required_reviewers(&pull_request, &[("ollie".into(), "ollie".into(), 10)])
                .unwrap_err()
                .to_string(),
            "Pull request 42 was updated, but reviewer 'ollie' now shows vote 5 instead of 10; check it with 'ado pr show'."
        );
    }

    #[test]
    fn validate_reviewers_present_matches_case_insensitively_and_reports_missing() {
        let pull_request = json!({
            "pullRequestId": 42,
            "reviewers": [{"id": "OLLIE"}],
        });
        assert!(
            validate_reviewers_present(
                &pull_request,
                &[("ollie".into(), "ollie".into())],
                "updated"
            )
            .is_ok()
        );
        assert_eq!(
            validate_reviewers_present(
                &pull_request,
                &[("grace".into(), "grace".into())],
                "updated"
            )
            .unwrap_err()
            .to_string(),
            "Pull request 42 was updated, but reviewer 'grace' is missing from the readback."
        );
        assert_eq!(
            validate_reviewers_present(
                &pull_request,
                &[("grace".into(), "grace".into())],
                "created"
            )
            .unwrap_err()
            .to_string(),
            "Pull request 42 was created, but reviewer 'grace' is missing from the readback."
        );
    }
}
