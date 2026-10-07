mod common;

use common::*;

#[test]
fn parses_supported_azure_remotes() {
    assert_eq!(
        AzureGitRemote::parse("https://dev.azure.com/acme/My%20Project/_git/service").unwrap(),
        AzureGitRemote::new("acme", "My Project", "service", None).unwrap()
    );
    let m = AzureGitRemote::parse("git@ssh.dev.azure.com:v3/acme/My%20Project/service").unwrap();
    assert_eq!(m.organization, "acme");
    assert_eq!(m.project, "My Project");
    assert_eq!(
        m.ssh_url.as_deref(),
        Some("git@ssh.dev.azure.com:v3/acme/My%20Project/service")
    );
    assert_eq!(
        AzureGitRemote::parse("acme@vs-ssh.visualstudio.com:v3/acme/project/repo")
            .unwrap()
            .repository,
        "repo"
    )
}
#[test]
fn rejects_untrusted_and_mismatched_remotes() {
    for v in [
        "git@github.com:acme/repo.git",
        "https://user@example.com/org/project/_git/repo",
        "https://dev.azure.com/acme//project/_git/repo",
        "https://dev.azure.com:443/acme/project/_git/repo",
        "other@vs-ssh.visualstudio.com:v3/acme/project/repo",
    ] {
        assert!(AzureGitRemote::parse(v).is_err())
    }
    let e = AzureGitRemote::new("acme", "p", "r", None).unwrap();
    assert!(
        AzureGitRemote::validated_ssh("git@ssh.dev.azure.com:v3/acme/p/other", Some(&e)).is_err()
    )
}
#[test]
fn pull_request_info_rejects_arbitrary_ssh_url() {
    let v = json!({"pullRequestId":7,"repository":{"name":"repo","sshUrl":"git@attacker.test:repo","project":{"name":"project"}},"sourceRefName":"refs/heads/topic","targetRefName":"refs/heads/main","lastMergeSourceCommit":{"commitId":"a".repeat(40)},"lastMergeTargetCommit":{"commitId":"b".repeat(40)}});
    assert!(PullRequestGitInfo::from_json(&v).is_err())
}
#[derive(Default)]
struct Recording {
    calls: Mutex<Vec<Vec<String>>>,
    destination: Mutex<Option<PathBuf>>,
    fetch: Mutex<usize>,
    status: Mutex<String>,
}
impl GitRunning for Recording {
    fn run(&self, a: &[String], _: Option<&Path>) -> Result<String> {
        self.calls.lock().unwrap().push(a.to_vec());
        if a.iter().any(|x| x == "clone") {
            if let Some(d) = self.destination.lock().unwrap().clone() {
                fs::create_dir_all(d.join(".git")).unwrap()
            }
            return Ok("".into());
        }
        if a.iter().any(|x| x == "fetch") {
            *self.fetch.lock().unwrap() += 1;
            return Ok("".into());
        }
        if a.iter().any(|x| x == "status") {
            return Ok(self.status.lock().unwrap().clone());
        }
        if a == svec(&["rev-parse", "FETCH_HEAD"]) {
            return Ok(if *self.fetch.lock().unwrap() == 1 {
                "b".repeat(40)
            } else {
                "a".repeat(40)
            });
        }
        if a == svec(&["rev-parse", "HEAD"]) {
            return Ok("a".repeat(40));
        }
        if a.first().map(String::as_str) == Some("merge-base") {
            return Ok("c".repeat(40));
        }
        Ok("".into())
    }
}
fn fixture() -> PullRequestGitInfo {
    let r =
        AzureGitRemote::validated_ssh("git@ssh.dev.azure.com:v3/acme/project/repo", None).unwrap();
    PullRequestGitInfo {
        id: 7,
        target: r.clone(),
        target_ssh_url: r.ssh_url.clone().unwrap(),
        source: r.clone(),
        source_ssh_url: r.ssh_url.clone().unwrap(),
        source_ref: "refs/heads/topic".into(),
        target_ref: "refs/heads/main".into(),
        source_commit: "a".repeat(40),
        target_commit: "b".repeat(40),
    }
}
#[test]
fn checkout_uses_arrays_and_protects_existing_directory() {
    let t = TempDir::new().unwrap();
    let occupied = t.path().join("occupied");
    fs::create_dir(&occupied).unwrap();
    let r = Recording::default();
    let c = ReviewCheckout::new(r);
    assert!(c.prepare(&fixture(), Some(occupied)).is_err());
    assert!(c.runner.calls.lock().unwrap().is_empty());
    let dest = t.path().join("managed");
    *c.runner.destination.lock().unwrap() = Some(dest.clone());
    let result = c.prepare(&fixture(), Some(dest.clone())).unwrap();
    assert_eq!(result.source_commit, "a".repeat(40));
    assert!(dest.join(".git/ado-review.json").exists());
    let calls = c.runner.calls.lock().unwrap();
    assert_eq!(
        calls[0],
        vec![
            "-c",
            "core.hooksPath=/dev/null",
            "clone",
            "--no-checkout",
            "git@ssh.dev.azure.com:v3/acme/project/repo",
            dest.to_str().unwrap(),
        ]
    );
    assert!(calls.contains(&svec(&[
        "-c",
        "core.hooksPath=/dev/null",
        "fetch",
        "--no-tags",
        "git@ssh.dev.azure.com:v3/acme/project/repo",
        "refs/heads/topic",
    ])));
    assert!(calls.contains(&svec(&[
        "-c",
        "core.hooksPath=/dev/null",
        "checkout",
        "--detach",
        &"a".repeat(40),
    ])))
}
#[test]
fn managed_checkout_rejects_dirty_working_tree() {
    let t = TempDir::new().unwrap();
    let dest = t.path().join("managed");
    let c = ReviewCheckout::new(Recording::default());
    *c.runner.destination.lock().unwrap() = Some(dest.clone());
    c.prepare(&fixture(), Some(dest.clone())).unwrap();
    *c.runner.status.lock().unwrap() = " M Sources/App.swift".into();
    assert!(c.prepare(&fixture(), Some(dest)).is_err())
}
struct Dictionary(HashMap<String, String>);
impl GitRunning for Dictionary {
    fn run(&self, a: &[String], _: Option<&Path>) -> Result<String> {
        self.0
            .get(&a.join(" "))
            .cloned()
            .ok_or_else(|| Error("missing mock".into()))
    }
}
#[test]
fn local_context_prefers_upstream_and_rejects_ambiguous() {
    let mut v = HashMap::from([
        ("rev-parse --abbrev-ref HEAD".into(), "topic".into()),
        ("remote".into(), "origin\nupstream".into()),
        (
            "remote get-url origin".into(),
            "git@ssh.dev.azure.com:v3/acme/project/fork".into(),
        ),
        (
            "remote get-url upstream".into(),
            "git@ssh.dev.azure.com:v3/acme/project/main".into(),
        ),
        ("config --get branch.topic.remote".into(), "upstream".into()),
    ]);
    assert_eq!(
        LocalGitContext::discover(&Dictionary(v.clone()), None)
            .unwrap()
            .remote
            .repository,
        "main"
    );
    v.remove("config --get branch.topic.remote");
    assert!(LocalGitContext::discover(&Dictionary(v), None).is_err())
}
#[test]
fn git_runner_drains_large_output_without_deadlock() {
    let t = TempDir::new().unwrap();
    let g = GitRunner;
    g.run(&svec(&["init"]), Some(t.path())).unwrap();
    let p = t.path().join("large.txt");
    let before = (0..80_000)
        .map(|i| format!("before-{i}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&p, &before).unwrap();
    g.run(&svec(&["add", "large.txt"]), Some(t.path())).unwrap();
    g.run(
        &svec(&[
            "-c",
            "user.name=ADO Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-m",
            "fixture",
        ]),
        Some(t.path()),
    )
    .unwrap();
    fs::write(p, before.replace("before-", "after-")).unwrap();
    let o = g
        .run(
            &svec(&["diff", "--no-ext-diff", "--no-textconv"]),
            Some(t.path()),
        )
        .unwrap();
    assert!(o.len() > 1_000_000)
}
