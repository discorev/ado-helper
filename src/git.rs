use crate::{Error, Result};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use url::Url;

pub fn is_commit_sha(v: &str) -> bool {
    v.len() == 40 && v.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn validate_ref(v: &str) -> Result<()> {
    let bad = !v.starts_with("refs/heads/")
        || v.len() <= "refs/heads/".len()
        || v.contains("..")
        || v.contains("@{")
        || v.contains('\\')
        || v.contains("//")
        || v.ends_with('/')
        || v.ends_with('.')
        || v == "@"
        || v.chars()
            .any(|c| c <= ' ' || c == '\u{7f}' || "~^:?*[".contains(c))
        || v.split('/')
            .any(|p| p.starts_with('.') || p.ends_with(".lock"));
    if bad {
        Err(Error("Unsafe Git value: invalid branch ref".into()))
    } else {
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureGitRemote {
    pub organization: String,
    pub project: String,
    pub repository: String,
    pub ssh_url: Option<String>,
}
impl AzureGitRemote {
    pub fn new(org: &str, project: &str, repo: &str, ssh: Option<&str>) -> Result<Self> {
        let base = Self {
            organization: component(org, "organization")?,
            project: component(project, "project")?,
            repository: component(repo, "repository")?,
            ssh_url: None,
        };
        if let Some(s) = ssh {
            let p = Self::parse(s)?;
            if !p.organization.eq_ignore_ascii_case(&base.organization)
                || p.project != base.project
                || p.repository != base.repository
            {
                return Err(invalid_remote());
            }
            Ok(Self {
                ssh_url: Some(s.into()),
                ..base
            })
        } else {
            Ok(base)
        }
    }
    pub fn parse(raw: &str) -> Result<Self> {
        if let Some(tail) = raw.strip_prefix("git@ssh.dev.azure.com:v3/") {
            return parse_scp(raw, tail, None);
        }
        if let Some(at) = raw.find('@') {
            let user = &raw[..at];
            let prefix = format!("{user}@vs-ssh.visualstudio.com:v3/");
            if let Some(tail) = raw.strip_prefix(&prefix) {
                return parse_scp(raw, tail, Some(user));
            }
        }
        if raw.to_ascii_lowercase().starts_with("ssh://") {
            let u = Url::parse(raw).map_err(|_| invalid_remote())?;
            if u.password().is_some()
                || u.query().is_some()
                || u.fragment().is_some()
                || u.port().is_some_and(|p| p != 22)
                || u.path().trim_start_matches('/').contains("//")
            {
                return Err(invalid_remote());
            }
            let host = u.host_str().unwrap_or("").to_ascii_lowercase();
            let parts = decode_path(u.path())?;
            if host == "vs-ssh.visualstudio.com"
                && parts.len() == 3
                && parts[1].eq_ignore_ascii_case("_git")
                && !u.username().is_empty()
            {
                return unchecked(u.username(), &parts[0], &parts[2], raw);
            }
            if host == "ssh.dev.azure.com"
                && u.username() == "git"
                && parts.len() == 4
                && parts[0].eq_ignore_ascii_case("v3")
            {
                return unchecked(&parts[1], &parts[2], &parts[3], raw);
            }
            return Err(invalid_remote());
        }
        let u = Url::parse(raw).map_err(|_| invalid_remote())?;
        if u.scheme() != "https"
            || !u.username().is_empty()
            || u.password().is_some()
            || u.port().is_some()
            || explicit_port(raw)
            || u.query().is_some()
            || u.fragment().is_some()
            || u.path().trim_start_matches('/').contains("//")
        {
            return Err(invalid_remote());
        }
        let host = u.host_str().unwrap_or("").to_ascii_lowercase();
        let p = decode_path(u.path())?;
        if host == "dev.azure.com" && p.len() == 4 && p[2].eq_ignore_ascii_case("_git") {
            return Self::new(&p[0], &p[1], &p[3], None);
        }
        if let Some(org) = host.strip_suffix(".visualstudio.com")
            && !org.is_empty()
            && !org.contains('.')
            && p.len() == 3
            && p[1].eq_ignore_ascii_case("_git")
        {
            return Self::new(org, &p[0], &p[2], None);
        }
        Err(invalid_remote())
    }
    pub fn validated_ssh(raw: &str, expected: Option<&Self>) -> Result<Self> {
        let p = Self::parse(raw)?;
        if p.ssh_url.is_none() {
            return Err(invalid_remote());
        }
        if let Some(e) = expected
            && (!p.organization.eq_ignore_ascii_case(&e.organization)
                || p.project != e.project
                || p.repository != e.repository)
        {
            return Err(invalid_remote());
        }
        Ok(p)
    }
}
fn parse_scp(raw: &str, tail: &str, legacy: Option<&str>) -> Result<AzureGitRemote> {
    if tail.contains([':', '?', '#']) {
        return Err(invalid_remote());
    }
    let raw_parts: Vec<&str> = tail.split('/').collect();
    if raw_parts.len() != 3 {
        return Err(invalid_remote());
    }
    let p: Vec<String> = raw_parts.into_iter().map(decode).collect::<Result<_>>()?;
    if legacy.is_some_and(|u| !u.eq_ignore_ascii_case(&p[0])) {
        return Err(invalid_remote());
    }
    unchecked(&p[0], &p[1], &p[2], raw)
}
fn unchecked(o: &str, p: &str, r: &str, raw: &str) -> Result<AzureGitRemote> {
    Ok(AzureGitRemote {
        organization: component(o, "organization")?,
        project: component(p, "project")?,
        repository: component(r, "repository")?,
        ssh_url: Some(raw.into()),
    })
}
fn component(v: &str, label: &str) -> Result<String> {
    if v.is_empty() || v == "." || v == ".." || v.chars().any(|c| "/\\:\0\n\r".contains(c)) {
        Err(Error(format!("Unsafe Git value: invalid {label}")))
    } else {
        Ok(v.into())
    }
}
fn decode_path(p: &str) -> Result<Vec<String>> {
    p.split('/').filter(|x| !x.is_empty()).map(decode).collect()
}
fn decode(v: &str) -> Result<String> {
    let x = percent_decode_str(v)
        .decode_utf8()
        .map_err(|_| invalid_remote())?
        .into_owned();
    if x.is_empty() {
        Err(invalid_remote())
    } else {
        Ok(x)
    }
}
fn invalid_remote() -> Error {
    Error("Unsupported, mismatched, or unsafe Azure Git remote.".into())
}

pub trait GitRunning {
    fn run(&self, args: &[String], directory: Option<&Path>) -> Result<String>;
}
#[derive(Debug, Clone, Copy, Default)]
pub struct GitRunner;
impl GitRunning for GitRunner {
    fn run(&self, args: &[String], directory: Option<&Path>) -> Result<String> {
        let mut c = Command::new("/usr/bin/git");
        c.args(args);
        if let Some(d) = directory {
            c.current_dir(d);
        }
        let out = c
            .output()
            .map_err(|e| Error(format!("Unable to start Git: {e}")))?;
        if !out.status.success() {
            let d = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return Err(Error(if d.is_empty() {
                "Git command failed.".into()
            } else {
                format!("Git failed: {d}")
            }));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalGitContext {
    pub remote: AzureGitRemote,
    pub branch: String,
}
impl LocalGitContext {
    pub fn discover<R: GitRunning>(runner: &R, directory: Option<&Path>) -> Result<Self> {
        let branch=runner.run(&s(&["rev-parse","--abbrev-ref","HEAD"]),directory).map_err(|_|Error("No Git repository was found. Supply --profile with a PR number, or use a PR URL.".into()))?;
        if branch.is_empty() || branch == "HEAD" {
            return Err(Error(
                "The current repository is in detached HEAD state.".into(),
            ));
        }
        let names = runner.run(&s(&["remote"]), directory).unwrap_or_default();
        let mut supported = Vec::new();
        for name in names.lines() {
            if let Ok(url) =
                runner.run(&["remote".into(), "get-url".into(), name.into()], directory)
                && let Ok(r) = AzureGitRemote::parse(&url)
            {
                supported.push((name.to_owned(), r));
            }
        }
        if let Ok(up) = runner.run(
            &[
                "config".into(),
                "--get".into(),
                format!("branch.{branch}.remote"),
            ],
            directory,
        ) && up != "."
        {
            if let Some((_, r)) = supported.iter().find(|(n, _)| n == &up) {
                return Ok(Self {
                    remote: r.clone(),
                    branch,
                });
            }
            return Err(invalid_remote());
        }
        let ids: HashSet<String> = supported
            .iter()
            .map(|(_, r)| {
                format!(
                    "{}\0{}\0{}",
                    r.organization.to_ascii_lowercase(),
                    r.project.to_ascii_lowercase(),
                    r.repository.to_ascii_lowercase()
                )
            })
            .collect();
        if ids.len() > 1 {
            return Err(invalid_remote());
        }
        let selected = supported
            .iter()
            .find(|(n, _)| n == "origin")
            .or_else(|| supported.first())
            .ok_or_else(invalid_remote)?;
        Ok(Self {
            remote: selected.1.clone(),
            branch,
        })
    }
}
fn s(values: &[&str]) -> Vec<String> {
    values.iter().map(|x| (*x).into()).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestGitInfo {
    pub id: i64,
    pub target: AzureGitRemote,
    pub target_ssh_url: String,
    pub source: AzureGitRemote,
    pub source_ssh_url: String,
    pub source_ref: String,
    pub target_ref: String,
    pub source_commit: String,
    pub target_commit: String,
}
impl PullRequestGitInfo {
    pub fn from_json(v: &Value) -> Result<Self> {
        let id = v
            .get("pullRequestId")
            .and_then(Value::as_i64)
            .ok_or_else(|| missing("pullRequestId"))?;
        let repo = v.get("repository").ok_or_else(|| missing("repository"))?;
        let (target, target_ssh_url) = repo_info(repo)?;
        let source_repo = v.pointer("/forkSource/repository").unwrap_or(repo);
        let (source, source_ssh_url) = repo_info(source_repo)?;
        if !target
            .organization
            .eq_ignore_ascii_case(&source.organization)
        {
            return Err(invalid_remote());
        }
        let source_ref = str_at(v, "sourceRefName")?;
        let target_ref = str_at(v, "targetRefName")?;
        let source_commit = v
            .pointer("/lastMergeSourceCommit/commitId")
            .and_then(Value::as_str)
            .ok_or_else(|| missing("source/target refs and commits"))?;
        let target_commit = v
            .pointer("/lastMergeTargetCommit/commitId")
            .and_then(Value::as_str)
            .ok_or_else(|| missing("source/target refs and commits"))?;
        validate_ref(source_ref)?;
        validate_ref(target_ref)?;
        if !is_commit_sha(source_commit) || !is_commit_sha(target_commit) {
            return Err(Error(
                "Unsafe Git value: invalid commit SHA from API".into(),
            ));
        }
        Ok(Self {
            id,
            target,
            target_ssh_url,
            source,
            source_ssh_url,
            source_ref: source_ref.into(),
            target_ref: target_ref.into(),
            source_commit: source_commit.to_ascii_lowercase(),
            target_commit: target_commit.to_ascii_lowercase(),
        })
    }
}
fn repo_info(v: &Value) -> Result<(AzureGitRemote, String)> {
    let pn = v
        .pointer("/project/name")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("repository project/name/sshUrl"))?;
    let rn = v
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("repository project/name/sshUrl"))?;
    let ssh = v
        .get("sshUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("repository project/name/sshUrl"))?;
    let remote = AzureGitRemote::validated_ssh(ssh, None)?;
    let project_ok = [Some(pn), v.pointer("/project/id").and_then(Value::as_str)]
        .into_iter()
        .flatten()
        .any(|x| x.eq_ignore_ascii_case(&remote.project));
    let repo_ok = [Some(rn), v.get("id").and_then(Value::as_str)]
        .into_iter()
        .flatten()
        .any(|x| x.eq_ignore_ascii_case(&remote.repository));
    if !project_ok || !repo_ok {
        return Err(invalid_remote());
    }
    Ok((remote, ssh.into()))
}
fn str_at<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| missing("source/target refs and commits"))
}
fn missing(f: &str) -> Error {
    Error(format!(
        "Azure DevOps pull request response is missing {f}."
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCheckoutResult {
    pub directory: String,
    pub source_commit: String,
    pub target_commit: String,
    pub merge_base: String,
}
pub struct ReviewCheckout<R: GitRunning = GitRunner> {
    pub runner: R,
}
impl Default for ReviewCheckout<GitRunner> {
    fn default() -> Self {
        Self { runner: GitRunner }
    }
}
impl<R: GitRunning> ReviewCheckout<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }
    pub fn prepare(
        &self,
        info: &PullRequestGitInfo,
        requested: Option<PathBuf>,
    ) -> Result<ReviewCheckoutResult> {
        let dir = requested.unwrap_or_else(|| {
            home()
                .join("Developer/ado-reviews")
                .join(&info.target.organization)
                .join(format!("{}-pr-{}", info.target.repository, info.id))
        });
        let marker = dir.join(".git/ado-review.json");
        let expected = Marker::from(info);
        if dir.exists() {
            let data = fs::read(&marker).ok();
            let existing = data
                .as_deref()
                .and_then(|d| serde_json::from_slice::<Marker>(d).ok());
            if existing.as_ref() != Some(&expected) {
                return Err(Error(format!(
                    "Refusing to use {}: it is not an ado-managed review directory for this pull request.",
                    dir.display()
                )));
            }
            let status = self.runner.run(
                &s(&[
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "core.fsmonitor=false",
                    "status",
                    "--porcelain",
                    "--untracked-files=all",
                ]),
                Some(&dir),
            )?;
            if !status.is_empty() {
                return Err(Error("Managed review checkout has local changes. Commit, move, or remove them before refreshing it.".into()));
            }
        } else {
            if let Some(parent) = dir.parent() {
                fs::create_dir_all(parent)?
            }
            self.runner.run(
                &[
                    "-c".into(),
                    "core.hooksPath=/dev/null".into(),
                    "clone".into(),
                    "--no-checkout".into(),
                    info.target_ssh_url.clone(),
                    dir.to_string_lossy().into_owned(),
                ],
                None,
            )?;
            fs::write(&marker, serde_json::to_vec(&expected)?)?
        }
        self.fetch(
            &info.target_ssh_url,
            &info.target_ref,
            &info.target_commit,
            &dir,
        )?;
        self.fetch(
            &info.source_ssh_url,
            &info.source_ref,
            &info.source_commit,
            &dir,
        )?;
        self.runner.run(
            &[
                "-c".into(),
                "core.hooksPath=/dev/null".into(),
                "checkout".into(),
                "--detach".into(),
                info.source_commit.clone(),
            ],
            Some(&dir),
        )?;
        let head = self
            .runner
            .run(&s(&["rev-parse", "HEAD"]), Some(&dir))?
            .to_ascii_lowercase();
        if head != info.source_commit {
            return Err(Error(
                "Checked out commit does not match the reviewed source commit.".into(),
            ));
        }
        let mb = self.runner.run(
            &[
                "merge-base".into(),
                info.target_commit.clone(),
                info.source_commit.clone(),
            ],
            Some(&dir),
        )?;
        if !is_commit_sha(&mb) {
            return Err(Error("Git returned an invalid merge base.".into()));
        }
        Ok(ReviewCheckoutResult {
            directory: dir.to_string_lossy().into_owned(),
            source_commit: info.source_commit.clone(),
            target_commit: info.target_commit.clone(),
            merge_base: mb.to_ascii_lowercase(),
        })
    }
    fn fetch(&self, url: &str, reference: &str, expected: &str, dir: &Path) -> Result<()> {
        self.runner.run(
            &[
                "-c".into(),
                "core.hooksPath=/dev/null".into(),
                "fetch".into(),
                "--no-tags".into(),
                url.into(),
                reference.into(),
            ],
            Some(dir),
        )?;
        let got = self
            .runner
            .run(&s(&["rev-parse", "FETCH_HEAD"]), Some(dir))?
            .to_ascii_lowercase();
        if got != expected {
            Err(Error("Fetched ref no longer matches the reviewed commit. Refresh the pull request and retry.".into()))
        } else {
            Ok(())
        }
    }
    pub fn diff(&self, r: &ReviewCheckoutResult) -> Result<String> {
        self.runner.run(
            &[
                "diff".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                r.merge_base.clone(),
                r.source_commit.clone(),
                "--".into(),
            ],
            Some(Path::new(&r.directory)),
        )
    }
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Marker {
    organization: String,
    project: String,
    repository: String,
    #[serde(rename = "pullRequestID")]
    pull_request_id: i64,
}
impl From<&PullRequestGitInfo> for Marker {
    fn from(i: &PullRequestGitInfo) -> Self {
        Self {
            organization: i.target.organization.clone(),
            project: i.target.project.clone(),
            repository: i.target.repository.clone(),
            pull_request_id: i.id,
        }
    }
}
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn explicit_port(value: &str) -> bool {
    value
        .split_once("://")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
        .and_then(|authority| authority.rsplit('@').next())
        .is_some_and(|host| host.contains(':'))
}
