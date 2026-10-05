use ado_core::{
    Error, Result,
    ado_client::{AdoClient, HttpRequest, HttpResponse, HttpTransport},
    auth::{self, AuthOps},
    cli::{self, CliCommand, PullRequestTarget},
    comment_anchor,
    git::*,
    keychain::{CredentialStore, KeychainStore},
    models::AdoIdentity,
    organization::{Organization, PrLocator},
    profiles::{Profile, ProfileStore},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tempfile::TempDir;

fn profile(name: &str, org: &str, id: &str) -> Profile {
    Profile {
        name: name.into(),
        organization: Organization::parse(org).unwrap(),
        identity: AdoIdentity {
            id: id.into(),
            display_name: "Test User".into(),
            unique_name: "test@example.com".into(),
        },
    }
}

// CLIParsingTests (4)
#[test]
fn parses_comment_with_explicit_review_identity() {
    let a = svec(&[
        "pr",
        "comment",
        "42",
        "--profile",
        "work",
        "--file",
        "/Sources/App.swift",
        "--line",
        "8",
        "--end-line",
        "10",
        "--side",
        "right",
        "--body-file",
        "/tmp/body",
        "--commit",
        &"a".repeat(40),
        "--iteration",
        "3",
        "--change-id",
        "17",
    ]);
    assert_eq!(
        cli::parse(&a).unwrap(),
        CliCommand::PrComment {
            target: PullRequestTarget::Number(42),
            profile: Some("work".into()),
            file: "/Sources/App.swift".into(),
            line: 8,
            end_line: 10,
            side: "right".into(),
            body_file: "/tmp/body".into(),
            commit: "a".repeat(40),
            iteration: 3,
            change_id: 17
        }
    )
}
#[test]
fn omitted_target_means_current_branch() {
    assert_eq!(
        cli::parse(&svec(&["pr", "show"])).unwrap(),
        CliCommand::PrShow {
            target: PullRequestTarget::CurrentBranch,
            profile: None
        }
    );
    assert_eq!(
        cli::parse(&svec(&["pr", "clone", "--directory", "/tmp/review"])).unwrap(),
        CliCommand::PrClone {
            target: PullRequestTarget::CurrentBranch,
            profile: None,
            directory: Some("/tmp/review".into())
        }
    )
}
#[test]
fn rejects_implicit_comment_identity_and_duplicate_options() {
    assert!(
        cli::parse(&svec(&[
            "pr",
            "comment",
            "1",
            "--file",
            "a",
            "--line",
            "1",
            "--side",
            "right",
            "--body-file",
            "b"
        ]))
        .is_err()
    );
    assert!(
        cli::parse(&svec(&[
            "pr",
            "show",
            "1",
            "--profile",
            "a",
            "--profile",
            "b"
        ]))
        .is_err()
    )
}
#[test]
fn auth_grammar() {
    assert_eq!(
        cli::parse(&svec(&[
            "auth",
            "add",
            "work",
            "--org",
            "https://dev.azure.com/acme",
            "--no-browser"
        ]))
        .unwrap(),
        CliCommand::AuthAdd {
            name: "work".into(),
            organization: "https://dev.azure.com/acme".into(),
            open_browser: false
        }
    );
    assert_eq!(
        cli::parse(&svec(&["auth", "status"])).unwrap(),
        CliCommand::AuthStatus { check: false }
    );
    assert!(cli::parse(&svec(&["auth", "add", "work"])).is_err());
    assert!(cli::parse(&svec(&["auth", "add", "work", "--org", "a", "--org", "b"])).is_err())
}

// CommentAnchorTests (6)
#[test]
fn addition_and_undelete_only_have_right_side() {
    assert!(comment_anchor::validate("left", 1, 1, "new\n", Some(&json!("add"))).is_err());
    assert!(comment_anchor::validate("right", 1, 1, "new\n", Some(&json!("add, edit"))).is_ok());
    assert!(
        comment_anchor::validate("left", 1, 1, "restored", Some(&json!("undelete, delete")))
            .is_err()
    );
    assert!(comment_anchor::validate("right", 1, 1, "restored", Some(&json!("undelete"))).is_ok())
}
#[test]
fn deletion_only_has_left_side() {
    assert!(comment_anchor::validate("right", 1, 1, "old", Some(&json!("delete"))).is_err());
    assert!(comment_anchor::validate("left", 1, 1, "old", Some(&json!("delete"))).is_ok())
}
#[test]
fn numeric_azure_change_flags_are_supported() {
    assert!(comment_anchor::validate_side("left", Some(&json!(1))).is_err());
    assert!(comment_anchor::validate_side("right", Some(&json!(16))).is_err());
    assert!(comment_anchor::validate_side("right", Some(&json!(32))).is_ok());
    assert!(comment_anchor::validate_side("left", Some(&json!(32))).is_err())
}
#[test]
fn edited_files_have_both_sides_and_missing_metadata_fails_closed() {
    assert!(comment_anchor::validate_side("left", Some(&json!("edit"))).is_ok());
    assert!(comment_anchor::validate_side("right", Some(&json!("edit"))).is_ok());
    assert!(comment_anchor::validate_side("right", None).is_err())
}
#[test]
fn line_count_does_not_invent_line_after_terminal_newline() {
    assert_eq!(comment_anchor::line_count(""), 0);
    assert_eq!(comment_anchor::line_count("one"), 1);
    assert_eq!(comment_anchor::line_count("one\n"), 1);
    assert_eq!(comment_anchor::line_count("one\ntwo"), 2);
    assert_eq!(comment_anchor::line_count("one\ntwo\n"), 2);
    assert!(comment_anchor::validate("right", 1, 2, "one\ntwo\n", Some(&json!("edit"))).is_ok());
    assert!(comment_anchor::validate("right", 2, 3, "one\ntwo\n", Some(&json!("edit"))).is_err())
}
#[test]
fn empty_file_has_no_valid_positive_anchor() {
    assert!(comment_anchor::validate("right", 1, 1, "", Some(&json!("add"))).is_err())
}

// APIOrganizationTests (5)
#[test]
fn organization_normalizes_supported_forms() {
    let b = Organization::parse("Example-Org").unwrap();
    assert_eq!(b.name, "example-org");
    assert_eq!(b.url(), "https://dev.azure.com/example-org");
    let modern = Organization::parse("https://dev.azure.com/example-org/").unwrap();
    assert_eq!(modern.name, "example-org");
    assert_eq!(modern.url(), "https://dev.azure.com/example-org");
    let legacy = Organization::parse("https://LegacyOrg.visualstudio.com/").unwrap();
    assert_eq!(legacy.name, "legacyorg");
    assert_eq!(legacy.url(), "https://dev.azure.com/legacyorg")
}
#[test]
fn organization_rejects_ambiguous_or_unsafe_inputs() {
    for v in [
        "http://dev.azure.com/org",
        "https://user:password@dev.azure.com/org",
        "https://dev.azure.com:443/org",
        "https://dev.azure.com/org/project",
        "https://dev.azure.com/org?token=secret",
        "https://dev.azure.com/org#fragment",
        "https://sub.org.visualstudio.com",
        "bad/name",
        "-bad",
        "bad-",
        " bad",
    ] {
        assert!(Organization::parse(v).is_err(), "{v}")
    }
}
#[test]
fn organization_codable_decoding_revalidates_and_normalizes() {
    let n: Organization = serde_json::from_str("{\"name\":\"FABRIKAM\"}").unwrap();
    assert_eq!(n.name, "fabrikam");
    assert!(
        serde_json::from_str::<Organization>("{\"name\":\"https://evil.example/org\"}").is_err()
    )
}
#[test]
fn pr_locator_parses_modern_and_legacy_urls() {
    let m = PrLocator::parse("https://dev.azure.com/acme/My%20Project/_git/Repo/pullrequest/42")
        .unwrap();
    assert_eq!(m.organization, Organization::parse("acme").unwrap());
    assert_eq!(m.project.as_deref(), Some("My Project"));
    assert_eq!(m.repository.as_deref(), Some("Repo"));
    assert_eq!(m.id, 42);
    let l =
        PrLocator::parse("https://acme.visualstudio.com/Project/_git/Repo/pullrequest/7/").unwrap();
    assert_eq!(l.organization, Organization::parse("acme").unwrap());
    assert_eq!(l.project.as_deref(), Some("Project"));
    assert_eq!(l.repository.as_deref(), Some("Repo"));
    assert_eq!(l.id, 7)
}
#[test]
fn pr_locator_rejects_non_pr_and_encoded_path_separator() {
    for v in [
        "http://dev.azure.com/acme/Project/_git/Repo/pullrequest/1",
        "https://evil.example/acme/Project/_git/Repo/pullrequest/1",
        "https://dev.azure.com/acme/Project/_git/Repo/pullrequest/0",
        "https://dev.azure.com/acme/Project%2FExtra/_git/Repo/pullrequest/1",
    ] {
        assert!(PrLocator::parse(v).is_err(), "{v}")
    }
}

// AuthStorageTests (5)
#[test]
fn profile_store_round_trips_without_token_and_uses_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new().unwrap();
    let s = ProfileStore::new(Some(d.path().join("store"))).unwrap();
    let p = profile("work", "example", "user-1");
    s.save(&p).unwrap();
    assert_eq!(s.load().unwrap(), vec![p.clone()]);
    assert_eq!(s.profile_by_name("work").unwrap(), p);
    assert_eq!(
        s.profile_by_organization(&Organization::parse("example").unwrap())
            .unwrap(),
        p
    );
    let text = fs::read_to_string(s.directory.join("profiles.json")).unwrap();
    assert!(!text.contains("secret-token"));
    assert_eq!(
        fs::metadata(&s.directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(s.directory.join("profiles.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    )
}
#[test]
fn profile_store_rejects_duplicate_organization() {
    let d = TempDir::new().unwrap();
    let s = ProfileStore::new(Some(d.path().join("s"))).unwrap();
    s.save(&profile("first", "example", "1")).unwrap();
    assert_eq!(
        s.save(&profile("second", "example", "2"))
            .unwrap_err()
            .to_string(),
        "A profile already exists for Azure DevOps organization 'example'."
    )
}
#[test]
fn profile_store_validates_names_before_using_them() {
    let d = TempDir::new().unwrap();
    let s = ProfileStore::new(Some(d.path().join("s"))).unwrap();
    for n in ["", ".hidden", "../escape", "has space", &"a".repeat(65)] {
        assert_eq!(
            s.save(&profile(n, "example", "1")).unwrap_err().to_string(),
            format!(
                "Invalid profile name '{n}'. Use 1-64 letters, numbers, dots, underscores, or hyphens, starting with a letter or number."
            )
        )
    }
}
#[test]
fn profile_store_rejects_organization_bypassing_initializer() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new().unwrap();
    let s = ProfileStore::new(Some(d.path().join("s"))).unwrap();
    let p = s.directory.join("profiles.json");
    fs::write(&p,r#"[{"identity":{"displayName":"Test","id":"user-1","uniqueName":"test@example.com"},"name":"work","organization":{"name":"../not-an-org"}}]"#).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(s.load().is_err())
}
#[test]
fn keychain_account_separates_organization_and_identity() {
    let a = profile("one", "alpha", "same-id");
    let b = profile("two", "beta", "same-id");
    let c = profile("three", "alpha", "other-id");
    assert_ne!(KeychainStore::account(&a), KeychainStore::account(&b));
    assert_ne!(KeychainStore::account(&a), KeychainStore::account(&c));
    assert!(KeychainStore::account(&a).contains("alpha"));
    assert!(KeychainStore::account(&a).contains("same-id"))
}

// AuthManagerTests (12)
#[derive(Default)]
struct AuthState {
    profiles: Vec<Profile>,
    saved: Vec<Profile>,
    tokens: Vec<(String, Profile)>,
    removed: Vec<Profile>,
    urls: Vec<String>,
    messages: Vec<String>,
    prompts: Vec<String>,
    validations: usize,
    interactive: usize,
    cleanup: usize,
    removals: usize,
    token: String,
    identity: Option<AdoIdentity>,
    confirm: bool,
    save_fail: bool,
    cleanup_fail: bool,
    interactive_fail: bool,
    remove_fail: bool,
    validation_fail: bool,
}
impl AuthOps for AuthState {
    fn load_profiles(&mut self) -> Result<Vec<Profile>> {
        Ok(self.profiles.clone())
    }
    fn save_profile(&mut self, p: &Profile) -> Result<()> {
        if self.save_fail {
            return Err(Error("fail".into()));
        }
        self.saved.push(p.clone());
        self.profiles.push(p.clone());
        Ok(())
    }
    fn remove_profile(&mut self, n: &str) -> Result<()> {
        self.removals += 1;
        if self.remove_fail {
            return Err(Error("fail".into()));
        }
        self.profiles.retain(|p| p.name != n);
        Ok(())
    }
    fn save_token(&mut self, t: &str, p: &Profile) -> Result<()> {
        self.tokens.push((t.into(), p.clone()));
        Ok(())
    }
    fn remove_token(&mut self, p: &Profile) -> Result<()> {
        self.cleanup += 1;
        if self.cleanup_fail {
            return Err(Error("fail".into()));
        }
        self.removed.push(p.clone());
        Ok(())
    }
    fn read_secret(&mut self, _: &str) -> Result<String> {
        Ok(self.token.clone())
    }
    fn confirm(&mut self, p: &str) -> Result<bool> {
        self.prompts.push(p.into());
        Ok(self.confirm)
    }
    fn require_interactive(&mut self) -> Result<()> {
        self.interactive += 1;
        if self.interactive_fail {
            Err(Error("Authentication needs an interactive terminal. Run this command directly in a terminal.".into()))
        } else {
            Ok(())
        }
    }
    fn open_url(&mut self, u: &str) -> Result<()> {
        self.urls.push(u.into());
        Ok(())
    }
    fn validate_identity(&mut self, _: &Organization, _: &str) -> Result<AdoIdentity> {
        self.validations += 1;
        if self.validation_fail {
            Err(Error("failure".into()))
        } else {
            Ok(self.identity.clone().unwrap())
        }
    }
    fn output(&mut self, m: &str) {
        self.messages.push(m.into())
    }
}
fn auth_state(token: &str, id: &str) -> AuthState {
    AuthState {
        token: token.into(),
        identity: Some(AdoIdentity {
            id: id.into(),
            display_name: "Example User".into(),
            unique_name: "user@example.com".into(),
        }),
        confirm: true,
        ..Default::default()
    }
}
#[test]
fn add_opens_pat_page_validates_and_saves_after_confirmation() {
    let mut s = auth_state(&"a".repeat(52), "identity-1");
    auth::add(
        &mut s,
        "work",
        Organization::parse("example").unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(
        s.urls,
        vec!["https://dev.azure.com/example/_usersSettings/tokens"]
    );
    assert_eq!(s.validations, 1);
    assert_eq!(
        s.tokens
            .iter()
            .map(|(token, _)| token.as_str())
            .collect::<Vec<_>>(),
        vec!["a".repeat(52)]
    );
    assert_eq!(
        s.saved
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        vec!["work"]
    );
    assert_eq!(s.tokens[0].1.identity, s.saved[0].identity);
    assert_eq!(s.prompts.len(), 1);
    assert!(s.prompts[0].contains("Example User"));
    assert!(s.prompts[0].contains("user@example.com"));
    assert!(s.messages.join("\n").contains("vso.threads_full"));
    assert!(!s.messages.join("\n").contains(&"a".repeat(52)))
}
#[test]
fn invalid_token_never_calls_network_or_storage() {
    let mut s = auth_state("short", "identity-1");
    assert_eq!(
        auth::add(
            &mut s,
            "work",
            Organization::parse("example").unwrap(),
            false
        )
        .unwrap_err()
        .to_string(),
        "The token is not plausible. Paste the complete PAT without spaces or line breaks."
    );
    assert_eq!(s.validations, 0);
    assert!(s.tokens.is_empty());
    assert!(s.saved.is_empty());
    assert!(s.urls.is_empty())
}
#[test]
fn update_with_different_identity_leaves_old_token_untouched() {
    let mut s = auth_state(&"b".repeat(52), "identity-2");
    s.profiles.push(profile("work", "example", "identity-1"));
    assert_eq!(
        auth::update(&mut s, "work", false).unwrap_err().to_string(),
        "The new token belongs to a different Azure DevOps identity. The existing token was not changed."
    );
    assert!(s.tokens.is_empty());
    assert!(s.prompts.is_empty())
}
#[test]
fn validation_failure_leaves_old_token_untouched() {
    let mut s = auth_state(&"c".repeat(52), "identity-1");
    s.profiles.push(profile("work", "example", "identity-1"));
    s.validation_fail = true;
    assert!(auth::update(&mut s, "work", false).is_err());
    assert!(s.tokens.is_empty());
    assert!(s.prompts.is_empty())
}
#[test]
fn add_rolls_back_keychain_when_profile_save_fails() {
    let mut s = auth_state(&"d".repeat(52), "identity-1");
    s.save_fail = true;
    assert_eq!(
        auth::add(
            &mut s,
            "work",
            Organization::parse("example").unwrap(),
            false
        )
        .unwrap_err()
        .to_string(),
        "Authentication succeeded, but the local profile could not be saved. No token was retained."
    );
    assert_eq!(s.tokens.len(), 1);
    assert_eq!(s.removed[0].name, "work")
}
#[test]
fn cancelled_confirmation_does_not_save() {
    let mut s = auth_state(&"e".repeat(52), "identity-1");
    s.confirm = false;
    assert_eq!(
        auth::add(
            &mut s,
            "work",
            Organization::parse("example").unwrap(),
            false
        )
        .unwrap_err()
        .to_string(),
        "Authentication was cancelled; no credentials were saved."
    );
    assert_eq!(s.validations, 1);
    assert!(s.tokens.is_empty());
    assert!(s.saved.is_empty())
}
#[test]
fn identity_confirmation_includes_names() {
    let mut s = auth_state(&"f".repeat(52), "identity-1");
    s.identity = Some(AdoIdentity {
        id: "identity-1".into(),
        display_name: "Alex Smith".into(),
        unique_name: "alex.one@example.com".into(),
    });
    auth::add(
        &mut s,
        "work",
        Organization::parse("example").unwrap(),
        false,
    )
    .unwrap();
    assert!(s.prompts[0].contains("Alex Smith <alex.one@example.com>"))
}
#[test]
fn noninteractive_fails_before_browser() {
    let mut s = auth_state(&"g".repeat(52), "identity-1");
    s.interactive_fail = true;
    assert_eq!(
        auth::add(
            &mut s,
            "work",
            Organization::parse("example").unwrap(),
            true
        )
        .unwrap_err()
        .to_string(),
        "Authentication needs an interactive terminal. Run this command directly in a terminal."
    );
    assert_eq!(s.interactive, 1);
    assert!(s.urls.is_empty());
    assert_eq!(s.validations, 0)
}
#[test]
fn profile_save_and_cleanup_failure_reports_retained_item() {
    let mut s = auth_state(&"h".repeat(52), "identity-1");
    s.save_fail = true;
    s.cleanup_fail = true;
    let e = auth::add(
        &mut s,
        "work",
        Organization::parse("example").unwrap(),
        false,
    )
    .unwrap_err();
    assert!(e.to_string().contains("Keychain"));
    assert_eq!(s.cleanup, 1);
    assert_eq!(s.tokens.len(), 1)
}
#[test]
fn remove_keychain_failure_preserves_profile() {
    let mut s = auth_state(&"i".repeat(52), "identity-1");
    s.profiles.push(profile("work", "example", "identity-1"));
    s.cleanup_fail = true;
    assert!(auth::remove(&mut s, "work").is_err());
    assert_eq!(s.profiles.len(), 1);
    assert_eq!(s.cleanup, 1);
    assert_eq!(s.removals, 0)
}
#[test]
fn remove_profile_failure_reports_token_gone() {
    let mut s = auth_state(&"j".repeat(52), "identity-1");
    s.profiles.push(profile("work", "example", "identity-1"));
    s.remove_fail = true;
    let e = auth::remove(&mut s, "work").unwrap_err();
    assert!(e.to_string().contains("Keychain token was removed"));
    assert!(e.to_string().contains("profile remains without a token"));
    assert_eq!(s.removed.len(), 1);
    assert_eq!(s.profiles.len(), 1);
    assert_eq!(s.removals, 1)
}
#[test]
fn scope_guidance_uses_least_privilege() {
    let o = Organization::parse("example").unwrap();
    let g = auth::scope_instructions(&o);
    assert!(g.contains("vso.code"));
    assert!(g.contains("vso.threads_full"));
    assert!(g.contains("vso.build"));
    assert!(g.contains("Do not choose Full access"));
    assert!(!g.contains("vso.code_write"));
    assert!(g.contains("organization policy"));
    assert_eq!(
        auth::pat_settings_url(&o),
        "https://dev.azure.com/example/_usersSettings/tokens"
    )
}

// GitTests (7)
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

// APIClientTests (13)
type Handler = Box<dyn Fn(&HttpRequest, usize) -> Result<HttpResponse> + Send + Sync>;
struct Mock {
    count: Mutex<usize>,
    handler: Handler,
}
impl Mock {
    fn new(
        f: impl Fn(&HttpRequest, usize) -> Result<HttpResponse> + Send + Sync + 'static,
    ) -> Self {
        Self {
            count: Mutex::new(0),
            handler: Box::new(f),
        }
    }
    fn count(&self) -> usize {
        *self.count.lock().unwrap()
    }
}
impl HttpTransport for Mock {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse> {
        let mut c = self.count.lock().unwrap();
        let i = *c;
        *c += 1;
        drop(c);
        (self.handler)(r, i)
    }
}
fn response(r: &HttpRequest, status: u16, json: &Value, headers: &[(&str, &str)]) -> HttpResponse {
    HttpResponse {
        status,
        url: r.url.clone(),
        headers: headers
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
        body: serde_json::to_vec(json).unwrap(),
    }
}
fn query(r: &HttpRequest) -> HashMap<String, String> {
    r.url
        .query_pairs()
        .map(|(a, b)| (a.into_owned(), b.into_owned()))
        .collect()
}
#[test]
fn identity_uses_account_property_fallback() {
    let m = Mock::new(|r, _| {
        assert_eq!(r.url.path(), "/acme/_apis/connectionData");
        assert_eq!(query(r)["connectOptions"], "1");
        assert_eq!(r.headers["Authorization"], "Basic OnRlc3QtdG9rZW4=");
        Ok(response(
            r,
            200,
            &json!({"authenticatedUser":{"id":"identity-id","providerDisplayName":"Ada Lovelace","properties":{"Account":{"$value":"ada@example.test"}}}}),
            &[],
        ))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "test-token", m);
    assert_eq!(
        c.identity().unwrap(),
        AdoIdentity {
            id: "identity-id".into(),
            display_name: "Ada Lovelace".into(),
            unique_name: "ada@example.test".into(),
        }
    );
    assert_eq!(c.transport.count(), 1)
}
#[test]
fn pull_request_uses_org_level_endpoint() {
    let m = Mock::new(|r, _| {
        assert_eq!(r.url.path(), "/acme/_apis/git/pullrequests/99");
        assert_eq!(query(r)["api-version"], "7.1");
        Ok(response(r, 200, &json!({"pullRequestId":99}), &[]))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    assert_eq!(c.pull_request(99).unwrap()["pullRequestId"], 99)
}
#[test]
fn file_content_pins_commit_and_escapes_path() {
    let commit = "ABCDEF0123456789ABCDEF0123456789ABCDEF01";
    let m = Mock::new(move |r, _| {
        assert_eq!(r.method, "GET");
        assert_eq!(r.headers["Accept"], "application/json");
        assert!(
            r.url
                .as_str()
                .contains("/My%20Project/_apis/git/repositories/repo%2Fname/items")
        );
        let q = query(r);
        assert_eq!(q["path"], "/Sources/A & B.swift");
        assert_eq!(q["includeContent"], "true");
        assert_eq!(q["includeContentMetadata"], "true");
        assert_eq!(q["versionDescriptor.versionType"], "commit");
        assert_eq!(q["versionDescriptor.version"], commit.to_ascii_lowercase());
        assert_eq!(q["api-version"], "7.1");
        Ok(response(
            r,
            200,
            &json!({"gitObjectType":"blob","isFolder":false,"contentMetadata":{"isBinary":false},"content":"first\nsecond\n"}),
            &[],
        ))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    assert_eq!(
        c.file_content("My Project", "repo/name", "Sources/A & B.swift", commit)
            .unwrap(),
        "first\nsecond\n"
    )
}
#[test]
fn file_content_rejects_binary_and_folder() {
    for p in [
        json!({"gitObjectType":"blob","isFolder":false,"contentMetadata":{"isBinary":true},"content":"AAEC"}),
        json!({"gitObjectType":"tree","isFolder":true,"contentMetadata":{"isBinary":false}}),
    ] {
        let m = Mock::new(move |r, _| Ok(response(r, 200, &p, &[])));
        let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
        assert_eq!(
            c.file_content("Project", "Repo", "/file.bin", &"a".repeat(40))
                .unwrap_err()
                .to_string(),
            "Azure DevOps returned a response in an unexpected format."
        )
    }
}
#[test]
fn file_content_rejects_invalid_commit_without_request() {
    let m = Mock::new(|_, _| panic!("request"));
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    assert_eq!(
        c.file_content("Project", "Repo", "/file.swift", "main")
            .unwrap_err()
            .to_string(),
        "Commit must be a 40-character hexadecimal SHA."
    );
    assert_eq!(c.transport.count(), 0)
}
#[test]
fn path_components_and_source_branch_escaped_as_data() {
    let m = Mock::new(|r, _| {
        assert!(
            r.url
                .as_str()
                .contains("/A%20Project/_apis/git/repositories/repo%2Fname/pullrequests")
        );
        assert_eq!(
            query(r)["searchCriteria.sourceRefName"],
            "refs/heads/feature & fix"
        );
        assert_eq!(query(r)["searchCriteria.status"], "active");
        Ok(response(r, 200, &json!({"value":[]}), &[]))
    });
    AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m)
        .list_pull_requests("A Project", "repo/name", Some("refs/heads/feature & fix"))
        .unwrap();
}
#[test]
fn changes_flattens_next_skip_pages() {
    let m = Mock::new(|r, i| {
        if i == 0 {
            let q = query(r);
            assert_eq!(q["$top"], "2000");
            assert!(!q.contains_key("$skip"));
            Ok(response(
                r,
                200,
                &json!({"changeEntries":[{"changeTrackingId":1}],"nextSkip":1,"nextTop":25}),
                &[],
            ))
        } else {
            let q = query(r);
            assert_eq!(q["$top"], "25");
            assert_eq!(q["$skip"], "1");
            Ok(response(
                r,
                200,
                &json!({"changeEntries":[{"changeTrackingId":2}],"nextSkip":0}),
                &[],
            ))
        }
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    let v = c.changes("Project", "Repo", 4, 3).unwrap();
    assert_eq!(
        v.iter()
            .map(|item| item["changeTrackingId"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(c.transport.count(), 2)
}
#[test]
fn threads_follows_continuation_header() {
    let token = "next token&opaque=yes";
    let m = Mock::new(move |r, i| {
        if i == 0 {
            assert!(!query(r).contains_key("continuationToken"));
            Ok(response(
                r,
                200,
                &json!({"value":[{"id":1}]}),
                &[("x-ms-continuationtoken", token)],
            ))
        } else {
            assert_eq!(query(r)["continuationToken"], token);
            Ok(response(r, 200, &json!({"value":[{"id":2}]}), &[]))
        }
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    let threads = c.threads("Project", "Repo", 4).unwrap();
    assert_eq!(
        threads
            .iter()
            .map(|item| item["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(c.transport.count(), 2)
}
#[test]
fn create_thread_builds_side_and_iteration_context() {
    let m = Mock::new(|r, _| {
        assert_eq!(r.method, "POST");
        let b: Value = serde_json::from_slice(r.body.as_ref().unwrap()).unwrap();
        assert_eq!(b["comments"][0]["content"], "Please adjust this");
        assert_eq!(b["threadContext"]["filePath"], "/Sources/App.swift");
        assert!(b["threadContext"]["leftFileStart"].is_null());
        assert_eq!(b["threadContext"]["rightFileStart"]["line"], 12);
        assert_eq!(b["threadContext"]["rightFileEnd"]["line"], 14);
        assert_eq!(b["pullRequestThreadContext"]["changeTrackingId"], 31);
        assert_eq!(
            b["pullRequestThreadContext"]["iterationContext"]["firstComparingIteration"],
            5
        );
        assert_eq!(
            b["pullRequestThreadContext"]["iterationContext"]["secondComparingIteration"],
            5
        );
        Ok(response(r, 200, &json!({"id":123}), &[]))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    assert_eq!(
        c.create_thread(
            "Project",
            "Repo",
            4,
            "Please adjust this",
            "/Sources/App.swift",
            12,
            14,
            "right",
            5,
            31
        )
        .unwrap()["id"],
        123
    );
    assert_eq!(c.transport.count(), 1)
}
#[test]
fn mutation_not_retried_and_error_hides_body_token() {
    let secret = "super-secret-token";
    let m = Mock::new(move |r, _| {
        Ok(HttpResponse {
            status: 503,
            url: r.url.clone(),
            headers: HashMap::new(),
            body: format!("private {secret}").into_bytes(),
        })
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), secret, m);
    let e = c
        .create_thread("Project", "Repo", 4, "body", "/file", 1, 1, "left", 1, 1)
        .unwrap_err()
        .to_string();
    assert!(e.contains("503"));
    assert!(!e.contains(secret));
    assert!(!e.contains("private"));
    assert_eq!(c.transport.count(), 1)
}
#[test]
fn cross_host_response_rejected_without_token() {
    let m = Mock::new(|_r, _| {
        Ok(HttpResponse {
            status: 200,
            url: url::Url::parse("https://evil.example/capture").unwrap(),
            headers: HashMap::new(),
            body: json!({"value":[]}).to_string().into_bytes(),
        })
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "redirect-secret", m);
    let e = c.threads("Project", "Repo", 4).unwrap_err().to_string();
    assert_eq!(
        e,
        "Azure DevOps redirected the request. Redirects are disabled for authenticated requests."
    );
    assert!(!e.contains("redirect-secret"))
}
#[test]
fn same_host_redirect_also_rejected() {
    let m = Mock::new(|r, _| {
        let mut u = r.url.clone();
        u.set_path("/another-organization/_apis/git/pullrequests/4");
        Ok(HttpResponse {
            status: 200,
            url: u,
            headers: HashMap::new(),
            body: json!({"pullRequestId":4}).to_string().into_bytes(),
        })
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "secret", m);
    assert_eq!(
        c.pull_request(4).unwrap_err().to_string(),
        "Azure DevOps redirected the request. Redirects are disabled for authenticated requests."
    )
}
#[test]
fn list_pull_requests_stops_repeating_full_page() {
    let page = Value::Array((0..100).map(|i| json!({"pullRequestId":i})).collect());
    let m = Mock::new(move |r, i| {
        assert_eq!(query(r)["$skip"], (i * 100).to_string());
        Ok(response(r, 200, &json!({"value":page}), &[]))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "secret", m);
    assert_eq!(
        c.list_pull_requests("Project", "Repo", None)
            .unwrap_err()
            .to_string(),
        "Azure DevOps returned an invalid or repeating pagination cursor."
    );
    assert_eq!(c.transport.count(), 2)
}

// IntegrationRegressionTests (6)
#[test]
fn full_pr_comment_uses_equal_iteration_context() {
    let m = Mock::new(|r, _| {
        let b: Value = serde_json::from_slice(r.body.as_ref().unwrap()).unwrap();
        assert_eq!(
            b["pullRequestThreadContext"]["iterationContext"]["firstComparingIteration"],
            4
        );
        assert_eq!(
            b["pullRequestThreadContext"]["iterationContext"]["secondComparingIteration"],
            4
        );
        Ok(response(r, 200, &json!({"id":1}), &[]))
    });
    AdoClient::with_transport(
        Organization::parse("example").unwrap(),
        "TEST_ONLY_NOT_A_PAT",
        m,
    )
    .create_thread(
        "Project",
        "Repo",
        42,
        "Review finding",
        "/budget.py",
        7,
        7,
        "left",
        4,
        12,
    )
    .unwrap();
}
#[test]
fn legacy_ssh_remote_form_recognised() {
    let r = AzureGitRemote::parse(
        "ssh://fabrikam@vs-ssh.visualstudio.com:22/Billing-Platform/_git/Payments_API",
    )
    .unwrap();
    assert_eq!(r.organization, "fabrikam");
    assert_eq!(r.project, "Billing-Platform");
    assert_eq!(r.repository, "Payments_API");
    assert!(r.ssh_url.is_some())
}
#[test]
fn diff_contains_actual_changes_between_commits() {
    let t = TempDir::new().unwrap();
    let g = GitRunner;
    g.run(&svec(&["init"]), Some(t.path())).unwrap();
    let p = t.path().join("budget.txt");
    fs::write(&p, "before\n").unwrap();
    g.run(&svec(&["add", "budget.txt"]), Some(t.path()))
        .unwrap();
    let commit = svec(&[
        "-c",
        "user.name=ADO Test",
        "-c",
        "user.email=ado-test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=/dev/null",
        "commit",
        "-m",
        "fixture",
    ]);
    g.run(&commit, Some(t.path())).unwrap();
    let base = g
        .run(&svec(&["rev-parse", "HEAD"]), Some(t.path()))
        .unwrap();
    fs::write(&p, "after\n").unwrap();
    g.run(&svec(&["add", "budget.txt"]), Some(t.path()))
        .unwrap();
    g.run(&commit, Some(t.path())).unwrap();
    let head = g
        .run(&svec(&["rev-parse", "HEAD"]), Some(t.path()))
        .unwrap();
    let d = ReviewCheckout::default()
        .diff(&ReviewCheckoutResult {
            directory: t.path().to_string_lossy().into_owned(),
            source_commit: head,
            target_commit: base.clone(),
            merge_base: base,
        })
        .unwrap();
    assert!(d.contains("-before"));
    assert!(d.contains("+after"))
}
#[test]
fn organisation_identity_case_insensitive_across_forms() {
    assert_eq!(
        Organization::parse("FABRIKAM").unwrap(),
        Organization::parse("https://fabrikam.visualstudio.com").unwrap()
    );
    assert_eq!(
        Organization::parse("https://dev.azure.com/Fabrikam/").unwrap(),
        Organization::parse("fabrikam").unwrap()
    )
}
#[test]
fn pasted_pr_link_may_include_files_tab() {
    let l=PrLocator::parse("https://dev.azure.com/fabrikam/Billing-Platform/_git/Payments_API/pullrequest/4321?_a=files&path=%2Fmain.py#discussion-123").unwrap();
    assert_eq!(l.organization.name, "fabrikam");
    assert_eq!(l.project.as_deref(), Some("Billing-Platform"));
    assert_eq!(l.repository.as_deref(), Some("Payments_API"));
    assert_eq!(l.id, 4321)
}
#[test]
fn legacy_default_collection_pr_links_resolve() {
    let l=PrLocator::parse("https://example.visualstudio.com/DefaultCollection/My%20Project/_git/Repo/pullrequest/42?_a=overview").unwrap();
    assert_eq!(l.organization.name, "example");
    assert_eq!(l.project.as_deref(), Some("My Project"));
    assert_eq!(l.id, 42)
}

#[test]
#[ignore = "requires an unlocked Secret Service session"]
fn secret_service_round_trip() {
    let p = profile("linux-roundtrip", "example", "secret-service-test");
    let k = KeychainStore;
    let token = "TEST_ONLY_SECRET_SERVICE_ROUNDTRIP";
    k.save(token, &p).unwrap();
    assert_eq!(k.read(&p).unwrap(), token);
    k.remove(&p).unwrap();
    assert!(k.read(&p).is_err())
}
fn svec(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn profile_load_uses_case_insensitive_natural_order() {
    let directory = TempDir::new().unwrap();
    let store = ProfileStore::new(Some(directory.path().join("profiles"))).unwrap();
    for (name, organization) in [
        ("Work", "work"),
        ("personal", "personal"),
        ("p10", "p-ten"),
        ("p9", "p-nine"),
    ] {
        store
            .save(&profile(name, organization, &format!("identity-{name}")))
            .unwrap();
    }
    assert_eq!(
        store
            .load()
            .unwrap()
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        vec!["p9", "p10", "personal", "Work"]
    );
}

#[test]
fn format_controls_are_rejected_or_neutralized() {
    assert_eq!(
        ado_core::platform::terminal_safe("safe\u{202e}name"),
        "safe�name"
    );
    assert_eq!(
        auth::validate_token(&format!("{}\u{200b}", "a".repeat(20)))
            .unwrap_err()
            .to_string(),
        "The token is not plausible. Paste the complete PAT without spaces or line breaks."
    );
    assert!(
        PrLocator::parse("https://dev.azure.com/acme/Project/_git/Repo%E2%80%AE/pullrequest/1")
            .is_err()
    );

    let transport = Mock::new(|_, _| panic!("invalid path must not issue a request"));
    let client =
        AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", transport);
    assert_eq!(
        client
            .file_content("Project", "Repo", "/safe\u{feff}name", &"a".repeat(40))
            .unwrap_err()
            .to_string(),
        "File path must be a valid repository-relative path."
    );
    assert_eq!(client.transport.count(), 0);
}

#[test]
fn http_status_error_decodes_request_path() {
    let transport = Mock::new(|request, _| Ok(response(request, 404, &json!({}), &[])));
    let client =
        AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", transport);
    let error = client
        .file_content("My Project", "Repo", "/file", &"a".repeat(40))
        .unwrap_err()
        .to_string();
    assert!(error.contains("/acme/My Project/_apis/"), "{error}");
    assert!(!error.contains("My%20Project"), "{error}");
}

#[test]
fn review_checkout_json_keys_match_sorted_swift_order() {
    let result = ReviewCheckoutResult {
        directory: "/tmp/review".into(),
        merge_base: "c".repeat(40),
        source_commit: "a".repeat(40),
        target_commit: "b".repeat(40),
    };
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(
        encoded.starts_with("{\"directory\":\"/tmp/review\",\"mergeBase\":"),
        "{encoded}"
    );
    assert!(encoded.find("mergeBase").unwrap() < encoded.find("sourceCommit").unwrap());
    assert!(encoded.find("sourceCommit").unwrap() < encoded.find("targetCommit").unwrap());
}

#[test]
fn non_utf8_arguments_report_a_normal_error() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, process::Command};
    let output = Command::new(env!("CARGO_BIN_EXE_ado"))
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: Command-line arguments must be valid UTF-8.\n"
    );
}

#[test]
fn closed_stdout_pipe_uses_default_sigpipe_behavior() {
    use std::{
        os::unix::process::ExitStatusExt,
        process::{Command, Stdio},
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_ado"))
        .arg("help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.signal(), Some(libc::SIGPIPE));
    assert!(output.stderr.is_empty());
}
