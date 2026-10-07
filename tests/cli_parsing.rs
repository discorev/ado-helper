mod common;

use common::*;

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
