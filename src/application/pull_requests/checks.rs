use super::*;
use ado_core::platform::terminal_safe;
use std::time::{Duration, Instant};

impl Application {
    pub(crate) fn pr_checks(
        &self,
        target: PullRequestTarget,
        wait: bool,
        timeout: u64,
        interval: u64,
        profile_name: Option<&str>,
    ) -> Result<i32> {
        let resolved = self.resolve(target, profile_name)?;
        let repository = repository_identity(&resolved.pull_request)?;
        let project_id = project_id(&resolved.pull_request)?;
        let pull_request_id = resolved.locator.id;
        let url = super::management::pull_request_url(
            &resolved.locator.organization,
            &repository.project,
            &repository.repository,
            pull_request_id,
        )?;
        let (state, evaluations) = if wait {
            let timeout = Duration::from_secs(timeout);
            let interval = Duration::from_secs(interval);
            let start = Instant::now();
            wait_for_checks(
                || {
                    resolved
                        .client
                        .policy_evaluations(&project_id, pull_request_id)
                },
                std::thread::sleep,
                || start.elapsed(),
                timeout,
                interval,
                |line| eprintln!("{}", terminal_safe(line)),
            )?
        } else {
            let evaluations = resolved
                .client
                .policy_evaluations(&project_id, pull_request_id)?;
            let state = overall_state(&evaluations).to_string();
            (state, evaluations)
        };
        let checks = evaluations
            .iter()
            .filter(|evaluation| enabled(evaluation))
            .map(|evaluation| {
                evaluation_output(
                    evaluation,
                    &resolved.locator.organization,
                    &repository.project,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        write_json(&json!({
            "pullRequestId": pull_request_id,
            "url": url,
            "state": state,
            "summary": summary(&evaluations),
            "checks": checks,
        }))?;
        Ok(exit_code(&state))
    }
}

/// Polls `fetch` for policy evaluations until the overall state is no longer `pending`, sleeping
/// between polls via `sleep` and tracking elapsed time via `elapsed` so tests can avoid real
/// time. Calls `notify` with one line per check whose status changed since the previous poll
/// (including the first poll). Never sleeps past `timeout`; if a poll after the final sleep is
/// still pending, the state is reported as `timedOut`.
pub(super) fn wait_for_checks(
    mut fetch: impl FnMut() -> Result<Vec<Value>>,
    mut sleep: impl FnMut(Duration),
    mut elapsed: impl FnMut() -> Duration,
    timeout: Duration,
    interval: Duration,
    mut notify: impl FnMut(&str),
) -> Result<(String, Vec<Value>)> {
    let mut previous: Vec<(String, String)> = Vec::new();
    loop {
        let evaluations = fetch()?;
        let current = evaluation_statuses(&evaluations);
        for (index, (name, status)) in current.iter().enumerate() {
            if previous
                .get(index)
                .map(|(_, previous_status)| previous_status)
                != Some(status)
            {
                notify(&format!("{name}: {status}"));
            }
        }
        previous = current;
        let state = overall_state(&evaluations);
        if state != "pending" {
            return Ok((state.to_string(), evaluations));
        }
        let remaining = timeout.saturating_sub(elapsed());
        if remaining.is_zero() {
            return Ok(("timedOut".into(), evaluations));
        }
        sleep(interval.min(remaining));
    }
}

fn exit_code(state: &str) -> i32 {
    match state {
        "rejected" => 2,
        "pending" | "timedOut" => 3,
        _ => 0,
    }
}

pub(super) fn project_id(pull_request: &Value) -> Result<String> {
    pull_request
        .pointer("/repository/project/id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            Error("Azure DevOps pull request response is missing the project ID.".into())
        })
}

fn enabled(evaluation: &Value) -> bool {
    evaluation
        .pointer("/configuration/isEnabled")
        .and_then(Value::as_bool)
        != Some(false)
}

fn blocking(evaluation: &Value) -> bool {
    evaluation
        .pointer("/configuration/isBlocking")
        .and_then(Value::as_bool)
        == Some(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Classification {
    Passed,
    Rejected,
    Pending,
}

/// Classifies a single evaluation for the purpose of the overall state and summary counts.
/// Returns `None` for a non-blocking evaluation, since it cannot affect either.
fn classify(evaluation: &Value) -> Option<Classification> {
    if !blocking(evaluation) {
        return None;
    }
    let status = evaluation
        .pointer("/status")
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(match status {
        "approved" | "notApplicable" => Classification::Passed,
        "rejected" | "broken" => Classification::Rejected,
        _ => Classification::Pending,
    })
}

fn classification_counts(evaluations: &[Value]) -> (i64, i64, i64) {
    let mut passed = 0;
    let mut rejected = 0;
    let mut pending = 0;
    for evaluation in evaluations {
        if !enabled(evaluation) {
            continue;
        }
        match classify(evaluation) {
            Some(Classification::Passed) => passed += 1,
            Some(Classification::Rejected) => rejected += 1,
            Some(Classification::Pending) => pending += 1,
            None => {}
        }
    }
    (passed, rejected, pending)
}

fn summary(evaluations: &[Value]) -> Value {
    let (passed, rejected, pending) = classification_counts(evaluations);
    json!({"passed": passed, "rejected": rejected, "pending": pending})
}

fn overall_state(evaluations: &[Value]) -> &'static str {
    let (_, rejected, pending) = classification_counts(evaluations);
    if rejected > 0 {
        "rejected"
    } else if pending > 0 {
        "pending"
    } else {
        "passed"
    }
}

fn evaluation_statuses(evaluations: &[Value]) -> Vec<(String, String)> {
    evaluations
        .iter()
        .filter(|evaluation| enabled(evaluation))
        .map(|evaluation| {
            let name = evaluation_name(evaluation).unwrap_or_else(|| "check".into());
            let status = evaluation
                .pointer("/status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            (name, status)
        })
        .collect()
}

fn nonempty_str(evaluation: &Value, pointer: &str) -> Option<String> {
    evaluation
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// name fallback order: settings.displayName, settings.defaultDisplayName,
/// context.buildDefinitionName, settings.statusGenre + "/" + settings.statusName (whichever of
/// the two is present), then configuration.type.displayName.
fn evaluation_name(evaluation: &Value) -> Option<String> {
    nonempty_str(evaluation, "/configuration/settings/displayName")
        .or_else(|| nonempty_str(evaluation, "/configuration/settings/defaultDisplayName"))
        .or_else(|| nonempty_str(evaluation, "/context/buildDefinitionName"))
        .or_else(|| {
            let genre = nonempty_str(evaluation, "/configuration/settings/statusGenre");
            let name = nonempty_str(evaluation, "/configuration/settings/statusName");
            match (genre, name) {
                (Some(genre), Some(name)) => Some(format!("{genre}/{name}")),
                (Some(genre), None) => Some(genre),
                (None, Some(name)) => Some(name),
                (None, None) => None,
            }
        })
        .or_else(|| nonempty_str(evaluation, "/configuration/type/displayName"))
}

fn evaluation_output(
    evaluation: &Value,
    organization: &Organization,
    project_name: &str,
) -> Result<Value> {
    let build_id = evaluation.pointer("/context/buildId").and_then(integer);
    let build_url = match build_id {
        Some(id) => Some(build_results_url(organization, project_name, id)?),
        None => None,
    };
    Ok(json!({
        "name": evaluation_name(evaluation).map(Value::String).unwrap_or(Value::Null),
        "type": evaluation.pointer("/configuration/type/displayName").cloned().unwrap_or(Value::Null),
        "blocking": blocking(evaluation),
        "status": evaluation.pointer("/status").and_then(Value::as_str).unwrap_or(""),
        "expired": evaluation.pointer("/context/isExpired").and_then(Value::as_bool) == Some(true),
        "buildId": build_id,
        "buildUrl": build_url,
    }))
}

fn build_results_url(organization: &Organization, project: &str, build_id: i64) -> Result<String> {
    let mut url = Url::parse(&organization.url())
        .map_err(|_| Error("Could not construct the build URL.".into()))?;
    url.path_segments_mut()
        .map_err(|_| Error("Could not construct the build URL.".into()))?
        .extend([project, "_build", "results"]);
    url.query_pairs_mut()
        .append_pair("buildId", &build_id.to_string());
    Ok(url.into())
}

/// If the policy-evaluation lookup succeeded and found enabled, blocking evaluations that have
/// not passed, appends their names and statuses to `error` and suggests `pr checks --wait`.
/// Otherwise returns `error` unchanged.
pub(super) fn completion_error_with_policies(
    error: Error,
    evaluations: Result<Vec<Value>>,
    pull_request_id: i64,
) -> Error {
    let Ok(evaluations) = evaluations else {
        return error;
    };
    let unpassed: Vec<String> = evaluations
        .iter()
        .filter(|evaluation| enabled(evaluation))
        .filter_map(|evaluation| match classify(evaluation) {
            Some(Classification::Passed) | None => None,
            Some(_) => {
                let name = evaluation_name(evaluation).unwrap_or_else(|| "check".into());
                let status = evaluation
                    .pointer("/status")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Some(format!(
                    "{} ({})",
                    terminal_safe(&name),
                    terminal_safe(status)
                ))
            }
        })
        .collect();
    if unpassed.is_empty() {
        return error;
    }
    Error(format!(
        "{error} Blocking policies not yet passed: {}. Run 'ado pr checks {pull_request_id} --wait' to wait for them.",
        unpassed.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluation(status: &str, blocking: bool, enabled: bool) -> Value {
        json!({
            "status": status,
            "configuration": {"isEnabled": enabled, "isBlocking": blocking},
        })
    }

    #[test]
    fn name_falls_back_through_settings_context_and_type() {
        let direct = json!({"configuration": {"settings": {"displayName": "API contracts"}}});
        assert_eq!(evaluation_name(&direct), Some("API contracts".into()));

        let default_display =
            json!({"configuration": {"settings": {"defaultDisplayName": "Default name"}}});
        assert_eq!(
            evaluation_name(&default_display),
            Some("Default name".into())
        );

        let build_name = json!({"context": {"buildDefinitionName": "CI"}});
        assert_eq!(evaluation_name(&build_name), Some("CI".into()));

        let genre_and_name =
            json!({"configuration": {"settings": {"statusGenre": "pr-ci", "statusName": "build"}}});
        assert_eq!(evaluation_name(&genre_and_name), Some("pr-ci/build".into()));

        let genre_only = json!({"configuration": {"settings": {"statusGenre": "pr-ci"}}});
        assert_eq!(evaluation_name(&genre_only), Some("pr-ci".into()));

        let name_only = json!({"configuration": {"settings": {"statusName": "build"}}});
        assert_eq!(evaluation_name(&name_only), Some("build".into()));

        let type_only = json!({"configuration": {"type": {"displayName": "Build"}}});
        assert_eq!(evaluation_name(&type_only), Some("Build".into()));

        assert_eq!(evaluation_name(&json!({})), None);

        // Empty strings are not treated as present; the first nonempty fallback wins.
        let mixed = json!({
            "configuration": {"settings": {"displayName": "", "defaultDisplayName": "Fallback"}},
        });
        assert_eq!(evaluation_name(&mixed), Some("Fallback".into()));
    }

    #[test]
    fn build_url_encodes_project_name_with_a_space() {
        let organization = Organization::parse("acme").unwrap();
        assert_eq!(
            build_results_url(&organization, "My Project", 42).unwrap(),
            "https://dev.azure.com/acme/My%20Project/_build/results?buildId=42"
        );
    }

    #[test]
    fn evaluation_output_resolves_build_url_only_when_build_id_present() {
        let organization = Organization::parse("acme").unwrap();
        let with_build = json!({
            "status": "running",
            "configuration": {"isEnabled": true, "isBlocking": true, "type": {"displayName": "Build"}, "settings": {"displayName": "API contracts"}},
            "context": {"buildId": 7, "isExpired": true},
        });
        let output = evaluation_output(&with_build, &organization, "My Project").unwrap();
        assert_eq!(output["name"], "API contracts");
        assert_eq!(output["type"], "Build");
        assert_eq!(output["blocking"], true);
        assert_eq!(output["status"], "running");
        assert_eq!(output["expired"], true);
        assert_eq!(output["buildId"], 7);
        assert_eq!(
            output["buildUrl"],
            "https://dev.azure.com/acme/My%20Project/_build/results?buildId=7"
        );

        let without_build =
            json!({"status": "queued", "configuration": {"isEnabled": true, "isBlocking": false}});
        let output = evaluation_output(&without_build, &organization, "My Project").unwrap();
        assert!(output["buildId"].is_null());
        assert!(output["buildUrl"].is_null());
        assert_eq!(output["blocking"], false);
    }

    #[test]
    fn disabled_evaluations_are_excluded_from_summary_and_state() {
        let evaluations = vec![
            evaluation("rejected", true, false),
            evaluation("approved", true, true),
        ];
        assert_eq!(overall_state(&evaluations), "passed");
        assert_eq!(
            summary(&evaluations),
            json!({"passed": 1, "rejected": 0, "pending": 0})
        );
    }

    #[test]
    fn classification_treats_optional_rejection_as_non_blocking() {
        let evaluations = vec![evaluation("rejected", false, true)];
        assert_eq!(overall_state(&evaluations), "passed");
        assert_eq!(
            summary(&evaluations),
            json!({"passed": 0, "rejected": 0, "pending": 0})
        );
    }

    #[test]
    fn not_applicable_passes_and_broken_rejects() {
        assert_eq!(
            classify(&evaluation("notApplicable", true, true)),
            Some(Classification::Passed)
        );
        assert_eq!(
            classify(&evaluation("broken", true, true)),
            Some(Classification::Rejected)
        );
        assert_eq!(
            classify(&evaluation("queued", true, true)),
            Some(Classification::Pending)
        );
        assert_eq!(
            classify(&evaluation("running", true, true)),
            Some(Classification::Pending)
        );
        assert_eq!(
            classify(&evaluation("weird", true, true)),
            Some(Classification::Pending)
        );
    }

    #[test]
    fn overall_state_prefers_rejected_over_pending() {
        let evaluations = vec![
            evaluation("rejected", true, true),
            evaluation("queued", true, true),
        ];
        assert_eq!(overall_state(&evaluations), "rejected");
    }

    #[test]
    fn wait_loop_passes_after_running_turns_approved() {
        let mut polls = vec![
            vec![evaluation("running", true, true)],
            vec![evaluation("approved", true, true)],
        ]
        .into_iter();
        let mut sleeps = Vec::new();
        let mut notifications = Vec::new();
        let mut elapsed = Duration::ZERO;
        let (state, evaluations) = wait_for_checks(
            || Ok(polls.next().unwrap()),
            |duration| sleeps.push(duration),
            || {
                elapsed += Duration::from_secs(5);
                elapsed
            },
            Duration::from_secs(60),
            Duration::from_secs(10),
            |line| notifications.push(line.to_string()),
        )
        .unwrap();
        assert_eq!(state, "passed");
        assert_eq!(evaluations.len(), 1);
        assert_eq!(sleeps, vec![Duration::from_secs(10)]);
        assert_eq!(notifications, vec!["check: running", "check: approved"]);
    }

    #[test]
    fn wait_loop_exits_early_on_rejection_without_sleeping() {
        let mut polls = vec![vec![evaluation("rejected", true, true)]].into_iter();
        let mut sleeps = Vec::new();
        let (state, _) = wait_for_checks(
            || Ok(polls.next().unwrap()),
            |duration| sleeps.push(duration),
            || Duration::ZERO,
            Duration::from_secs(60),
            Duration::from_secs(10),
            |_| {},
        )
        .unwrap();
        assert_eq!(state, "rejected");
        assert!(sleeps.is_empty());
    }

    #[test]
    fn wait_loop_times_out_and_never_sleeps_past_the_deadline() {
        let evaluations = vec![evaluation("queued", true, true)];
        let mut sleeps = Vec::new();
        let mut elapsed = Duration::ZERO;
        let (state, _) = wait_for_checks(
            || Ok(evaluations.clone()),
            |duration| sleeps.push(duration),
            || {
                let current = elapsed;
                elapsed += Duration::from_secs(8);
                current
            },
            Duration::from_secs(20),
            Duration::from_secs(10),
            |_| {},
        )
        .unwrap();
        assert_eq!(state, "timedOut");
        // First poll at t=0 sleeps min(10, 20)=10. Second poll at t=8 has 12s remaining, sleeps
        // min(10,12)=10. Third poll at t=16 has 4s remaining, sleeps only the remaining 4s. The
        // next poll at t=24 is still pending with 0s remaining, so it stops without sleeping again.
        assert_eq!(
            sleeps,
            vec![
                Duration::from_secs(10),
                Duration::from_secs(10),
                Duration::from_secs(4)
            ]
        );
        for sleep in &sleeps {
            assert!(*sleep <= Duration::from_secs(20));
        }
    }

    #[test]
    fn wait_loop_only_notifies_on_status_change() {
        let mut polls = vec![
            vec![evaluation("running", true, true)],
            vec![evaluation("running", true, true)],
            vec![evaluation("approved", true, true)],
        ]
        .into_iter();
        let mut notifications = Vec::new();
        wait_for_checks(
            || Ok(polls.next().unwrap()),
            |_| {},
            || Duration::ZERO,
            Duration::from_secs(60),
            Duration::from_secs(10),
            |line| notifications.push(line.to_string()),
        )
        .unwrap();
        assert_eq!(notifications, vec!["check: running", "check: approved"]);
    }

    #[test]
    fn completion_error_appends_blocking_policy_names_and_suggests_wait() {
        let error = Error("Azure DevOps returned HTTP 400.".into());
        let evaluations = vec![
            json!({
                "status": "running",
                "configuration": {"isEnabled": true, "isBlocking": true, "settings": {"displayName": "API contracts"}},
            }),
            json!({
                "status": "queued",
                "configuration": {"isEnabled": true, "isBlocking": true, "settings": {"displayName": "Minimum reviewers"}},
            }),
            json!({
                "status": "approved",
                "configuration": {"isEnabled": true, "isBlocking": true, "settings": {"displayName": "Already passed"}},
            }),
        ];
        let enriched = completion_error_with_policies(error.clone(), Ok(evaluations), 42);
        assert_eq!(
            enriched.to_string(),
            "Azure DevOps returned HTTP 400. Blocking policies not yet passed: API contracts (running), Minimum reviewers (queued). Run 'ado pr checks 42 --wait' to wait for them."
        );
    }

    #[test]
    fn completion_error_is_unchanged_when_lookup_fails_or_nothing_is_blocking() {
        let error = Error("Azure DevOps returned HTTP 400.".into());
        let failed =
            completion_error_with_policies(error.clone(), Err(Error("lookup failed".into())), 42);
        assert_eq!(failed, error);

        let nothing_blocking = completion_error_with_policies(
            error.clone(),
            Ok(vec![evaluation("approved", true, true)]),
            42,
        );
        assert_eq!(nothing_blocking, error);

        let disabled = completion_error_with_policies(
            error.clone(),
            Ok(vec![evaluation("rejected", true, false)]),
            42,
        );
        assert_eq!(disabled, error);
    }

    #[test]
    fn project_id_requires_a_nonempty_repository_project_id() {
        let pull_request =
            json!({"repository": {"project": {"id": "11111111-1111-1111-1111-111111111111"}}});
        assert_eq!(
            project_id(&pull_request).unwrap(),
            "11111111-1111-1111-1111-111111111111"
        );
        assert!(project_id(&json!({})).is_err());
        assert!(project_id(&json!({"repository": {"project": {"id": ""}}})).is_err());
    }
}
