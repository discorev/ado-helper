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

#[test]
fn parses_top_level_comment_without_file_anchor() {
    let commit = "a".repeat(40);
    assert_eq!(
        cli::parse(&svec(&[
            "pr",
            "comment",
            "42",
            "--top-level",
            "--profile",
            "work",
            "--body-file",
            "/tmp/body",
            "--commit",
            &commit
        ]))
        .unwrap(),
        CliCommand::PrTopLevelComment {
            target: PullRequestTarget::Number(42),
            profile: Some("work".into()),
            body_file: "/tmp/body".into(),
            commit: commit.clone()
        }
    );
    assert!(matches!(
        cli::parse(&svec(&[
            "pr",
            "comment",
            "--top-level",
            "--body-file",
            "/tmp/body",
            "--commit",
            &commit
        ]))
        .unwrap(),
        CliCommand::PrTopLevelComment {
            target: PullRequestTarget::CurrentBranch,
            ..
        }
    ));
}

#[test]
fn top_level_comment_rejects_missing_identity_and_mixed_anchors() {
    let commit = "a".repeat(40);
    let base = svec(&[
        "pr",
        "comment",
        "42",
        "--top-level",
        "--body-file",
        "/tmp/body",
        "--commit",
        &commit,
    ]);
    for (option, value) in [
        ("--file", "/file"),
        ("--line", "1"),
        ("--end-line", "2"),
        ("--side", "right"),
        ("--iteration", "1"),
        ("--change-id", "1"),
    ] {
        let mut args = base.clone();
        args.extend(svec(&[option, value]));
        assert!(cli::parse(&args).is_err(), "accepted {option}");
    }
    assert!(
        cli::parse(&svec(&[
            "pr",
            "comment",
            "--top-level",
            "--body-file",
            "/tmp/body"
        ]))
        .is_err()
    );
    assert!(
        cli::parse(&svec(&[
            "pr",
            "comment",
            "--top-level",
            "--commit",
            &commit
        ]))
        .is_err()
    );
    assert!(
        cli::parse(&svec(&[
            "pr",
            "comment",
            "--top-level",
            "--body-file",
            "/tmp/body",
            "--commit",
            "invalid"
        ]))
        .is_err()
    );
    let mut duplicate = base;
    duplicate.push("--top-level".into());
    assert!(cli::parse(&duplicate).is_err());
}

#[test]
fn parses_comment_edit_and_requires_expected_body() {
    let args = svec(&[
        "pr",
        "edit-comment",
        "42",
        "--thread-id",
        "7",
        "--comment-id",
        "1",
        "--body-file",
        "/tmp/new",
        "--expected-body-file",
        "/tmp/old",
    ]);
    assert_eq!(
        cli::parse(&args).unwrap(),
        CliCommand::PrEditComment {
            target: PullRequestTarget::Number(42),
            profile: None,
            thread_id: 7,
            comment_id: 1,
            body_file: "/tmp/new".into(),
            expected_body_file: "/tmp/old".into()
        }
    );
    assert!(cli::parse(&args[..args.len() - 2]).is_err());
    let mut zero = args.clone();
    zero[4] = "0".into();
    assert!(cli::parse(&zero).is_err());
}

#[test]
fn parses_pr_open_with_required_target() {
    assert_eq!(
        cli::parse(&svec(&["pr", "open", "42", "--profile", "work"])).unwrap(),
        CliCommand::PrOpen {
            target: PullRequestTarget::Number(42),
            profile: Some("work".into()),
        }
    );
    assert!(cli::parse(&svec(&["pr", "open"])).is_err());
}

#[test]
fn parses_pr_query_with_defaults_and_filters() {
    assert_eq!(
        cli::parse(&svec(&["pr", "query"])).unwrap(),
        CliCommand::PrQuery {
            status: "active".into(),
            creator: None,
            reviewer: None,
            source: None,
            target: None,
            top: 100,
            profile: None,
        }
    );
    assert_eq!(
        cli::parse(&svec(&[
            "pr",
            "query",
            "--status",
            "completed",
            "--creator",
            "me",
            "--reviewer",
            "ada@example.test",
            "--source",
            "topic",
            "--target",
            "main",
            "--top",
            "25",
            "--profile",
            "work",
        ]))
        .unwrap(),
        CliCommand::PrQuery {
            status: "completed".into(),
            creator: Some("me".into()),
            reviewer: Some("ada@example.test".into()),
            source: Some("topic".into()),
            target: Some("main".into()),
            top: 25,
            profile: Some("work".into()),
        }
    );
    for args in [
        svec(&["pr", "query", "42"]),
        svec(&["pr", "query", "--status", "invalid"]),
        svec(&["pr", "query", "--top", "0"]),
    ] {
        assert!(cli::parse(&args).is_err());
    }
}

#[test]
fn parses_pr_create_with_repeatable_reviewers_and_work_items() {
    assert_eq!(
        cli::parse(&svec(&[
            "pr",
            "create",
            "--source",
            "topic",
            "--target",
            "main",
            "--title",
            "Add search",
            "--description-file",
            "/tmp/description",
            "--draft",
            "--reviewer",
            "ada@example.test",
            "--reviewer",
            "grace@example.test",
            "--work-item",
            "12",
            "--work-item",
            "13",
            "--tags",
            "api,urgent",
            "--profile",
            "work",
        ]))
        .unwrap(),
        CliCommand::PrCreate {
            source: Some("topic".into()),
            target: Some("main".into()),
            title: "Add search".into(),
            description_file: Some("/tmp/description".into()),
            draft: true,
            reviewers: vec!["ada@example.test".into(), "grace@example.test".into()],
            work_items: vec![12, 13],
            tags: Some("api,urgent".into()),
            profile: Some("work".into()),
        }
    );
    assert!(cli::parse(&svec(&["pr", "create"])).is_err());
    assert!(
        cli::parse(&svec(&[
            "pr",
            "create",
            "--title",
            "Title",
            "--work-item",
            "0"
        ]))
        .is_err()
    );
}

#[test]
fn parses_pr_update_and_requires_a_field() {
    assert_eq!(
        cli::parse(&svec(&[
            "pr",
            "update",
            "42",
            "--title",
            "New title",
            "--description-file",
            "/tmp/description",
            "--status",
            "active",
            "--ready",
            "--add-reviewer",
            "ada@example.test",
            "--add-reviewer",
            "grace@example.test",
            "--tags",
            "api,ready",
            "--profile",
            "work",
        ]))
        .unwrap(),
        CliCommand::PrUpdate {
            target: PullRequestTarget::Number(42),
            title: Some("New title".into()),
            description_file: Some("/tmp/description".into()),
            status: Some("active".into()),
            draft: Some(false),
            add_reviewers: vec!["ada@example.test".into(), "grace@example.test".into()],
            tags: Some("api,ready".into()),
            profile: Some("work".into()),
        }
    );
    assert_eq!(
        cli::parse(&svec(&["pr", "update", "--draft"])).unwrap(),
        CliCommand::PrUpdate {
            target: PullRequestTarget::CurrentBranch,
            title: None,
            description_file: None,
            status: None,
            draft: Some(true),
            add_reviewers: vec![],
            tags: None,
            profile: None,
        }
    );
    for args in [
        svec(&["pr", "update"]),
        svec(&["pr", "update", "42", "--status", "completed"]),
        svec(&["pr", "update", "42", "--draft", "--ready"]),
    ] {
        assert!(cli::parse(&args).is_err());
    }
}

#[test]
fn parses_work_item_queries_and_validates_source_and_top() {
    assert_eq!(
        cli::parse(&svec(&[
            "wi",
            "query",
            "--search",
            "broken build",
            "--top",
            "25",
            "--profile",
            "work",
            "--project",
            "Web App",
        ]))
        .unwrap(),
        CliCommand::WiQuery {
            query: WorkItemQuery::Search("broken build".into()),
            top: 25,
            profile: Some("work".into()),
            project: Some("Web App".into()),
        }
    );
    assert_eq!(
        cli::parse(&svec(&["wi", "query", "--wiql-file", "/tmp/query"])).unwrap(),
        CliCommand::WiQuery {
            query: WorkItemQuery::WiqlFile("/tmp/query".into()),
            top: 100,
            profile: None,
            project: None,
        }
    );
    assert!(cli::parse(&svec(&["wi", "query"])).is_err());
    assert!(
        cli::parse(&svec(&[
            "wi",
            "query",
            "--wiql",
            "SELECT [System.Id] FROM WorkItems",
            "--search",
            "bug"
        ]))
        .is_err()
    );
    for top in ["0", "-1", "not-a-number"] {
        assert!(
            cli::parse(&svec(&["wi", "query", "--search", "bug", "--top", top])).is_err(),
            "accepted top {top}"
        );
    }
}

#[test]
fn parses_work_item_read_comment_and_open_commands() {
    assert_eq!(
        cli::parse(&svec(&[
            "wi",
            "show",
            "7",
            "--project",
            "Project",
            "--profile",
            "work"
        ]))
        .unwrap(),
        CliCommand::WiShow {
            id: 7,
            profile: Some("work".into()),
            project: Some("Project".into()),
        }
    );
    assert_eq!(
        cli::parse(&svec(&["wi", "comments", "8"])).unwrap(),
        CliCommand::WiComments {
            id: 8,
            profile: None,
            project: None,
        }
    );
    assert_eq!(
        cli::parse(&svec(&["wi", "comment", "9", "--body-file", "/tmp/body"])).unwrap(),
        CliCommand::WiComment {
            id: 9,
            body_file: "/tmp/body".into(),
            profile: None,
            project: None,
        }
    );
    assert_eq!(
        cli::parse(&svec(&["wi", "open", "10", "--project", "P"])).unwrap(),
        CliCommand::WiOpen {
            id: 10,
            profile: None,
            project: Some("P".into()),
        }
    );
    for args in [
        svec(&["wi", "show"]),
        svec(&["wi", "comments", "0"]),
        svec(&["wi", "comment", "1"]),
        svec(&["wi", "open", "invalid"]),
    ] {
        assert!(cli::parse(&args).is_err());
    }
}

#[test]
fn parses_work_item_create_and_requires_type_and_title() {
    assert_eq!(
        cli::parse(&svec(&[
            "wi",
            "create",
            "--type",
            "User Story",
            "--title",
            "Add search",
            "--description-file",
            "/tmp/description",
            "--assign-to",
            "ada@example.test",
            "--tags",
            "api;urgent"
        ]))
        .unwrap(),
        CliCommand::WiCreate {
            work_item_type: "User Story".into(),
            title: "Add search".into(),
            description_file: Some("/tmp/description".into()),
            assign_to: Some("ada@example.test".into()),
            tags: Some("api;urgent".into()),
            profile: None,
            project: None,
        }
    );
    assert!(cli::parse(&svec(&["wi", "create", "--title", "Title"])).is_err());
    assert!(cli::parse(&svec(&["wi", "create", "--type", "Bug"])).is_err());
}

#[test]
fn parses_work_item_update_and_requires_a_field() {
    assert_eq!(
        cli::parse(&svec(&[
            "wi",
            "update",
            "12",
            "--state",
            "Active",
            "--title",
            "New title",
            "--assign-to",
            "ada@example.test",
            "--tags",
            "one;two",
            "--description-file",
            "/tmp/description"
        ]))
        .unwrap(),
        CliCommand::WiUpdate {
            id: 12,
            state: Some("Active".into()),
            title: Some("New title".into()),
            assign_to: Some("ada@example.test".into()),
            tags: Some("one;two".into()),
            description_file: Some("/tmp/description".into()),
            profile: None,
            project: None,
        }
    );
    assert!(cli::parse(&svec(&["wi", "update", "12"])).is_err());
}
