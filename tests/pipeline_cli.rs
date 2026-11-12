mod common;
use ado_core::cli::PipelineTarget;
use common::*;

fn parse(args: &[&str]) -> Result<CliCommand> {
    cli::parse(&svec(args))
}

#[test]
fn pipeline_list_parses_filter_and_scope() {
    assert_eq!(
        parse(&[
            "pipeline",
            "list",
            "--name",
            "example-*",
            "--project",
            "Example Project",
            "--profile",
            "work"
        ])
        .unwrap(),
        CliCommand::PipelineList {
            name: Some("example-*".into()),
            project: Some("Example Project".into()),
            profile: Some("work".into())
        }
    );
    assert_eq!(
        parse(&["pipeline", "list"]).unwrap(),
        CliCommand::PipelineList {
            name: None,
            profile: None,
            project: None
        }
    );
}

#[test]
fn pipeline_runs_defaults_and_all_filters() {
    assert_eq!(
        parse(&["pipeline", "runs"]).unwrap(),
        CliCommand::PipelineRuns {
            pipeline: None,
            branch: None,
            status: None,
            result: None,
            top: 25,
            profile: None,
            project: None
        }
    );
    assert_eq!(
        parse(&[
            "pipeline",
            "runs",
            "--pipeline",
            "CI",
            "--branch",
            "main",
            "--status",
            "INPROGRESS",
            "--result",
            "partiallySucceeded",
            "--top",
            "7",
            "--project",
            "Example Project",
            "--profile",
            "work"
        ])
        .unwrap(),
        CliCommand::PipelineRuns {
            pipeline: Some("CI".into()),
            branch: Some("refs/heads/main".into()),
            status: Some("inProgress".into()),
            result: Some("partiallySucceeded".into()),
            top: 7,
            project: Some("Example Project".into()),
            profile: Some("work".into())
        }
    );
    assert!(
        matches!(parse(&["pipeline", "runs", "--pipeline", "4", "--branch", "refs/heads/main"]).unwrap(), CliCommand::PipelineRuns { pipeline: Some(pipeline), branch: Some(branch), .. } if pipeline == "4" && branch == "refs/heads/main")
    );
    for status in [
        "none",
        "inProgress",
        "completed",
        "cancelling",
        "postponed",
        "notStarted",
        "all",
    ] {
        assert!(parse(&["pipeline", "runs", "--status", status]).is_ok());
    }
    for result in [
        "none",
        "succeeded",
        "partiallySucceeded",
        "failed",
        "canceled",
    ] {
        assert!(parse(&["pipeline", "runs", "--result", result]).is_ok());
    }
}

#[test]
fn pipeline_run_commands_parse_ids_and_full_urls() {
    let target = PipelineTarget::Id(123);
    assert_eq!(
        parse(&[
            "pipeline",
            "show",
            "123",
            "--profile",
            "work",
            "--project",
            "Example Project"
        ])
        .unwrap(),
        CliCommand::PipelineShow {
            target: target.clone(),
            profile: Some("work".into()),
            project: Some("Example Project".into())
        }
    );
    assert_eq!(
        parse(&["pipeline", "open", "123"]).unwrap(),
        CliCommand::PipelineOpen {
            target: target.clone(),
            profile: None,
            project: None
        }
    );
    assert_eq!(
        parse(&["pipeline", "tests", "123"]).unwrap(),
        CliCommand::PipelineTests {
            target: target.clone(),
            all: false,
            profile: None,
            project: None
        }
    );
    assert_eq!(
        parse(&["pipeline", "tests", "--all", "123"]).unwrap(),
        CliCommand::PipelineTests {
            target,
            all: true,
            profile: None,
            project: None
        }
    );
    let url = "https://dev.azure.com/acme/Example%20Project/_build/results?buildId=123&view=logs";
    let target = PipelineTarget::Url {
        organization: Organization::parse("acme").unwrap(),
        project: "Example Project".into(),
        id: 123,
    };
    assert_eq!(
        parse(&["pipeline", "show", url]).unwrap(),
        CliCommand::PipelineShow {
            target: target.clone(),
            profile: None,
            project: None
        }
    );
    assert_eq!(target.id(), 123);
    for command in ["open", "tests"] {
        assert!(parse(&["pipeline", command, url]).is_ok());
    }
    assert!(parse(&["pipeline", "log", url, "--failed"]).is_ok());
}

#[test]
fn pipeline_log_modes_and_tail() {
    assert_eq!(
        parse(&["pipeline", "log", "123", "--log-id", "7", "--tail", "20"]).unwrap(),
        CliCommand::PipelineLog {
            target: PipelineTarget::Id(123),
            log_id: Some(7),
            failed: false,
            tail: Some(20),
            profile: None,
            project: None
        }
    );
    assert_eq!(
        parse(&[
            "pipeline",
            "log",
            "123",
            "--failed",
            "--tail",
            "0",
            "--profile",
            "work",
            "--project",
            "Example Project"
        ])
        .unwrap(),
        CliCommand::PipelineLog {
            target: PipelineTarget::Id(123),
            log_id: None,
            failed: true,
            tail: Some(0),
            profile: Some("work".into()),
            project: Some("Example Project".into())
        }
    );
    for args in [
        svec(&["pipeline", "log", "123"]),
        svec(&["pipeline", "log", "123", "--failed", "--log-id", "7"]),
        svec(&["pipeline", "log", "123", "--log-id", "0"]),
        svec(&["pipeline", "log", "123", "--log-id", "-1"]),
        svec(&["pipeline", "log", "123", "--failed", "--tail", "-1"]),
        svec(&["pipeline", "log", "123", "--failed", "--tail", "1.5"]),
        svec(&["pipeline", "log", "123", "--failed", "--failed"]),
        svec(&["pipeline", "tests", "123", "--all", "--all"]),
    ] {
        assert!(cli::parse(&args).is_err(), "{args:?}");
    }
}

#[test]
fn pipeline_validation_rejects_unknown_or_invalid_options_and_targets() {
    for args in [
        vec!["pipeline", "nope"],
        vec!["pipeline", "list", "123"],
        vec!["pipeline", "runs", "123"],
        vec!["pipeline", "runs", "--top", "0"],
        vec!["pipeline", "runs", "--top", "-1"],
        vec!["pipeline", "runs", "--top", "nope"],
        vec!["pipeline", "runs", "--status", "running"],
        vec!["pipeline", "runs", "--result", "cancelled"],
        vec!["pipeline", "runs", "--branch", ""],
        vec!["pipeline", "runs", "--branch", "bad..branch"],
        vec!["pipeline", "runs", "--pipeline", "0"],
        vec!["pipeline", "runs", "--pipeline", ""],
        vec!["pipeline", "list", "--profile", ""],
        vec!["pipeline", "list", "--project", ""],
        vec!["pipeline", "list", "--name"],
        vec!["pipeline", "list", "--name", "CI", "--name", "CI"],
        vec!["pipeline", "tests", "123", "--failed"],
        vec!["pipeline", "show", "123", "456"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
    for command in ["show", "log", "tests", "open"] {
        assert!(parse(&["pipeline", command]).is_err());
    }
    for target in [
        "0",
        "-1",
        "not-an-id",
        "9223372036854775808",
        "http://dev.azure.com/acme/Example%20Project/_build/results?buildId=1",
        "https://example.com/acme/Example%20Project/_build/results?buildId=1",
        "https://dev.azure.com.evil.test/acme/Example%20Project/_build/results?buildId=1",
        "https://ada@example.com@dev.azure.com/acme/Example%20Project/_build/results?buildId=1",
        "https://dev.azure.com:443/acme/Example%20Project/_build/results?buildId=1",
        "https://dev.azure.com/acme/Example%20Project/_build/results",
        "https://dev.azure.com/acme/Example%20Project/_build/results?buildId=0",
        "https://dev.azure.com/acme/Example%20Project/_build/results?buildId=-1",
        "https://dev.azure.com/acme/Example%20Project/_build/results?buildId=1&buildId=2",
        "https://dev.azure.com/acme/Example%2FProject/_build/results?buildId=1",
        "https://dev.azure.com/acme/Example%00Project/_build/results?buildId=1",
        "https://dev.azure.com/acme//_build/results?buildId=1",
        "https://dev.azure.com/acme/Example%20Project/_git/example-repo/pullrequest/1",
    ] {
        assert!(parse(&["pipeline", "show", target]).is_err(), "{target}");
    }
}

#[test]
fn pipeline_help_exits_successfully_without_accessing_profiles_or_running_commands() {
    for command in [
        None,
        Some("list"),
        Some("runs"),
        Some("show"),
        Some("log"),
        Some("tests"),
        Some("open"),
    ] {
        let mut args = vec!["pipeline"];
        if let Some(command) = command {
            args.push(command);
        }
        args.push("--help");
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_ado"))
            .args(&args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Usage:"));
        assert!(text.contains("ado pipeline"));
        assert!(!text.contains("  ado pr "));
        assert!(text.contains("--top defaults to 25"));
    }
}
