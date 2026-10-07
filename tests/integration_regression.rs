mod common;

use common::*;

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
