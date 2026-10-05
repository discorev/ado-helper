use ado_core::{
    Error, Result,
    ado_client::AdoClient,
    auth::{self, ProductionAuth},
    cli::{CliCommand, PullRequestTarget},
    comment_anchor,
    git::{
        AzureGitRemote, GitRunner, LocalGitContext, PullRequestGitInfo, ReviewCheckout,
        is_commit_sha,
    },
    keychain::{CredentialStore, KeychainStore},
    organization::{Organization, PrLocator},
    platform::terminal_safe,
    profiles::{Profile, ProfileStore},
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn main() {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    if let Err(e) = run() {
        eprintln!("error: {}", terminal_safe(&e.to_string()));
        std::process::exit(1)
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| Error("Command-line arguments must be valid UTF-8.".into()))
        })
        .collect::<Result<_>>()?;
    let command = ado_core::cli::parse(&args)?;
    if command == CliCommand::Help {
        println!("{HELP}");
        return Ok(());
    }
    let app = Application {
        store: ProfileStore::new(None)?,
        keychain: KeychainStore,
    };
    app.run(command)
}
struct Application {
    store: ProfileStore,
    keychain: KeychainStore,
}
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
    fn run(&self, c: CliCommand) -> Result<()> {
        match c {
            CliCommand::Help => println!("{HELP}"),
            CliCommand::AuthAdd {
                name,
                organization,
                open_browser,
            } => {
                let mut o = ProductionAuth {
                    store: &self.store,
                    keychain: &self.keychain,
                };
                auth::add(
                    &mut o,
                    &name,
                    Organization::parse(&organization)?,
                    open_browser,
                )?
            }
            CliCommand::AuthUpdate { name, open_browser } => {
                let mut o = ProductionAuth {
                    store: &self.store,
                    keychain: &self.keychain,
                };
                auth::update(&mut o, &name, open_browser)?
            }
            CliCommand::AuthStatus { check } => self.auth_status(check)?,
            CliCommand::AuthRemove { name } => {
                let mut o = ProductionAuth {
                    store: &self.store,
                    keychain: &self.keychain,
                };
                auth::remove(&mut o, &name)?
            }
            CliCommand::PrShow { target, profile } => {
                write_json(&self.resolve(target, profile.as_deref())?.pull_request)?
            }
            CliCommand::PrThreads { target, profile } => {
                let r = self.resolve(target, profile.as_deref())?;
                let repo = repository_identity(&r.pull_request)?;
                write_json(&Value::Array(r.client.threads(
                    &repo.project,
                    &repo.repository,
                    r.locator.id,
                )?))?
            }
            CliCommand::PrChanges {
                target,
                profile,
                iteration,
            } => {
                let r = self.resolve(target, profile.as_deref())?;
                let repo = repository_identity(&r.pull_request)?;
                let iterations =
                    r.client
                        .iterations(&repo.project, &repo.repository, r.locator.id)?;
                let iteration = requested_iteration(iteration, &iterations)?;
                let changes =
                    r.client
                        .changes(&repo.project, &repo.repository, r.locator.id, iteration)?;
                write_json(
                    &json!({"pullRequestId":r.locator.id,"iteration":iteration,"changes":changes}),
                )?
            }
            CliCommand::PrClone {
                target,
                profile,
                directory,
            } => {
                let r = self.resolve(target, profile.as_deref())?;
                let result = self.prepare_checkout(&r, directory)?;
                write_value(&result)?
            }
            CliCommand::PrDiff {
                target,
                profile,
                directory,
            } => {
                let r = self.resolve(target, profile.as_deref())?;
                let checkout = ReviewCheckout::default();
                let result = self.prepare_checkout_with(&r, directory, &checkout)?;
                let diff = checkout.diff(&result)?;
                write_json(&json!({"checkout":result,"diff":diff}))?
            }
            CliCommand::PrComment {
                target,
                profile,
                file,
                line,
                end_line,
                side,
                body_file,
                commit,
                iteration,
                change_id,
            } => self.comment(
                target,
                profile.as_deref(),
                &file,
                line,
                end_line,
                &side,
                &body_file,
                &commit,
                iteration,
                change_id,
            )?,
        }
        Ok(())
    }
    fn auth_status(&self, check: bool) -> Result<()> {
        let profiles = self.store.load()?;
        if profiles.is_empty() {
            println!("No authentication profiles configured.");
            return Ok(());
        }
        for p in profiles {
            if check {
                let token = self.keychain.read(&p)?;
                let id = AdoClient::new(p.organization.clone(), &token)?.identity()?;
                if id.id != p.identity.id {
                    return Err(Error(format!(
                        "Profile '{}' authenticated as a different identity.",
                        p.name
                    )));
                }
                println!(
                    "{}\t{}\t{}\tverified",
                    p.name,
                    p.organization.url(),
                    terminal_safe(&id.unique_name)
                );
            } else {
                println!(
                    "{}\t{}\t{}",
                    p.name,
                    p.organization.url(),
                    terminal_safe(&p.identity.unique_name)
                );
            }
        }
        Ok(())
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
    #[allow(clippy::too_many_arguments)]
    fn comment(
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
fn standardize_path(value: String) -> Result<PathBuf> {
    use std::path::Component;
    let path = PathBuf::from(value);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
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
fn read_body(path: &str) -> Result<String> {
    let m =
        fs::metadata(path).map_err(|_| Error("Could not read the comment body file.".into()))?;
    if m.len() > 1_048_576 {
        return Err(Error("Comment body file must be 1 MiB or smaller.".into()));
    }
    let data = fs::read(path).map_err(|_| Error("Could not read the comment body file.".into()))?;
    let body = String::from_utf8(data)
        .map_err(|_| Error("Comment body file must contain nonempty UTF-8 text.".into()))?;
    if body.trim().is_empty() {
        Err(Error(
            "Comment body file must contain nonempty UTF-8 text.".into(),
        ))
    } else {
        Ok(body)
    }
}
fn integer(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|x| i64::try_from(x).ok()))
}
fn write_json(v: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}
fn write_value<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}
const HELP: &str = "Usage:\n  ado auth add NAME --org URL [--no-browser]\n  ado auth update NAME [--no-browser]\n  ado auth status [--check]\n  ado auth remove NAME\n  ado pr show [TARGET] [--profile NAME]\n  ado pr threads [TARGET] [--profile NAME]\n  ado pr changes [TARGET] [--profile NAME] [--iteration N]\n  ado pr clone [TARGET] [--profile NAME] [--directory PATH]\n  ado pr diff [TARGET] [--profile NAME] [--directory PATH]\n  ado pr comment [TARGET] --file PATH --line N [--end-line N] --side left|right\n      --body-file PATH --commit SHA --iteration N --change-id N [--profile NAME]\n\nTARGET is an Azure DevOps PR URL or positive PR number. Omit it to use the current branch.\nPull-request data is written as JSON; authentication and help are human-readable.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_iteration_does_not_evaluate_latest_iteration() {
        assert_eq!(requested_iteration(Some(7), &[]).unwrap(), 7);
        assert_eq!(
            requested_iteration(None, &[]).unwrap_err().to_string(),
            "Azure DevOps pull request response is missing pull request iteration."
        );
    }
}
