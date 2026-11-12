use super::*;
use ado_core::ado_client::{HttpRequest, HttpResponse};
use std::{cell::Cell, collections::HashMap};

fn organization() -> Organization {
    Organization::parse("acme").unwrap()
}

fn timeline() -> Vec<Value> {
    vec![
        json!({"id":"stage", "type":"Stage", "name":"CI", "result":"failed"}),
        json!({"id":"phase", "parentId":"stage", "type":"Phase", "name":"Agent phase"}),
        json!({"id":"job", "parentId":"phase", "type":"Job", "name":"Linux", "result":"failed", "log":{"id":2}}),
        json!({"id":"task", "parentId":"job", "type":"Task", "name":"Test", "state":"completed", "result":"failed", "log":{"id":7}, "startTime":"2026-01-01T00:01:00Z", "finishTime":"2026-01-01T00:02:00Z", "issues":[{"type":"error", "message":"A test failed"}, {"type":"warning", "message":"Example warning"}, {"type":"info", "message":"Ignored"}]}),
        json!({"id":"ok", "parentId":"job", "type":"Task", "name":"Build", "result":"succeeded", "log":{"id":8}}),
        json!({"id":"other", "parentId":"job", "type":"Task", "name":"Publish", "result":"Failed", "log":{"id":9}}),
    ]
}

#[test]
fn definitions_include_yaml_repository_and_encoded_web_url() {
    let output = definition_output(&json!({"id":4, "name":"CI", "path":"\\Examples", "process":{"yamlFilename":"example-ci.yml"}, "repository":{"name":"example-repo"}}), &organization(), "Example Project").unwrap();
    assert_eq!(
        output,
        json!({"id":4, "name":"CI", "folder":"\\Examples", "yamlPath":"example-ci.yml", "repository":"example-repo", "url":"https://dev.azure.com/acme/Example%20Project/_build?definitionId=4"})
    );
    let classic = definition_output(
        &json!({"id":5, "name":"Classic"}),
        &organization(),
        "Example Project",
    )
    .unwrap();
    assert!(classic["yamlPath"].is_null());
    assert!(classic["repository"].is_null());
    assert!(definition_output(&json!({"id":0}), &organization(), "Example Project").is_err());
}

#[test]
fn run_summary_contains_all_requested_fields_and_nulls() {
    let build = json!({"id":123, "buildNumber":"20260101.1", "definition":{"id":4,"name":"CI"}, "status":"completed", "result":"failed", "sourceBranch":"refs/heads/main", "sourceVersion":"0123456789abcdef0123456789abcdef01234567", "reason":"individualCI", "requestedFor":{"displayName":"Ada Example", "uniqueName":"ada@example.com"}, "queueTime":"2026-01-01T00:00:00Z", "startTime":"2026-01-01T00:01:00Z", "finishTime":"2026-01-01T00:02:00Z"});
    let output = run_output(&build, &organization(), "Example Project").unwrap();
    assert_eq!(output["pipeline"], json!({"id":4,"name":"CI"}));
    assert_eq!(output["requestedFor"], "Ada Example");
    assert_eq!(output["sourceVersion"], build["sourceVersion"]);
    assert_eq!(
        output["url"],
        "https://dev.azure.com/acme/Example%20Project/_build/results?buildId=123"
    );
    for field in [
        "buildNumber",
        "status",
        "result",
        "sourceBranch",
        "reason",
        "queueTime",
        "startTime",
        "finishTime",
    ] {
        assert_eq!(output[field], build[field]);
    }
    let queued = run_output(&json!({"id":124}), &organization(), "Example Project").unwrap();
    assert!(queued["result"].is_null());
    assert!(queued["finishTime"].is_null());
}

#[test]
fn timeline_keeps_parent_nesting_and_error_warning_issues() {
    let output = timeline_output(&timeline()).unwrap();
    let rows = output.as_array().unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0]["parentId"], Value::Null);
    assert_eq!(rows[1]["parentId"], "stage");
    assert_eq!(rows[2]["parentId"], "job");
    assert_eq!(rows[2]["logId"], 7);
    assert_eq!(rows[2]["state"], "completed");
    assert_eq!(rows[2]["startTime"], "2026-01-01T00:01:00Z");
    assert_eq!(rows[2]["issues"].as_array().unwrap().len(), 2);
    assert_eq!(rows[2]["issues"][0]["message"], "A test failed");
    assert_eq!(rows[0]["issues"], json!([]));
    assert_eq!(timeline_output(&[]).unwrap(), json!([]));
}

#[test]
fn failed_logs_only_select_failed_tasks_with_stage_job_task_headers() {
    assert_eq!(
        failed_task_logs(&timeline()).unwrap(),
        vec![
            (Some(7), "CI / Linux / Test".into()),
            (Some(9), "CI / Linux / Publish".into())
        ]
    );
    assert!(failed_task_logs(&[]).unwrap().is_empty());
    let missing = vec![json!({"type":"Task", "name":"Test", "result":"failed"})];
    assert_eq!(
        failed_task_logs(&missing).unwrap(),
        vec![(None, "Test".into())]
    );
}

#[test]
fn cyclic_timeline_cannot_hang_log_path_resolution() {
    let records = vec![
        json!({"id":"a", "parentId":"b", "type":"Task", "result":"failed", "log":{"id":1}}),
        json!({"id":"b", "parentId":"a", "type":"Phase"}),
    ];
    assert!(failed_task_logs(&records).unwrap_err().0.contains("cyclic"));
    let records = vec![
        json!({"id":"task", "parentId":"phase", "type":"Task"}),
        json!({"id":"phase", "parentId":"phase", "type":"Phase"}),
    ];
    assert!(timeline_output(&records).unwrap_err().0.contains("cyclic"));
}

#[test]
fn tail_preserves_exact_text_and_handles_empty_zero_and_crlf() {
    for (text, count, expected) in [
        ("one\ntwo\nthree\n", Some(2), "two\nthree\n"),
        ("one\ntwo\nthree", Some(1), "three"),
        ("one\ntwo\n", Some(5), "one\ntwo\n"),
        ("one\r\ntwo\r\n", Some(1), "two\r\n"),
        ("\n\n", Some(1), "\n"),
        ("one\n", Some(0), ""),
        ("", Some(1), ""),
        ("one\n", None, "one\n"),
        ("héllo\n世界", Some(1), "世界"),
    ] {
        assert_eq!(tail_lines(text, count), expected);
    }
}

#[test]
fn test_counts_use_all_outcomes_while_results_keep_containing_run() {
    let run = json!({"id":11,"name":"example-ci tests", "state":"Completed", "totalTests":5});
    let statistics = json!({"runStatistics":[{"outcome":"Passed","count":2}, {"outcome":"Failed","count":1}, {"outcome":"NotExecuted","count":1}, {"outcome":"Blocked","count":1}]});
    let results = vec![
        json!({"id":22, "testCaseTitle":"example test", "automatedTestName":"ExampleTests.example", "outcome":"Failed", "errorMessage":"Example assertion", "stackTrace":"at ExampleTests.example", "durationInMs":12.5}),
    ];
    let output = test_run_output(&run, &statistics, &results).unwrap();
    assert_eq!(output["total"], 5);
    assert_eq!(output["passed"], 2);
    assert_eq!(output["failed"], 1);
    assert_eq!(output["notExecuted"], 1);
    assert_eq!(
        output["results"][0],
        json!({"id":22,"testName":"example test", "automatedTestName":"ExampleTests.example", "outcome":"Failed", "errorMessage":"Example assertion", "stackTrace":"at ExampleTests.example", "durationInMs":12.5,"testRun":{"id":11,"name":"example-ci tests"}})
    );
    assert!(test_run_output(&run, &json!({}), &results).is_err());
    let empty = test_run_output(&json!({"id":1}), &json!({"runStatistics":[]}), &[]).unwrap();
    assert_eq!(empty["total"], 0);
}

struct DefinitionTransport {
    body: Value,
    count: Cell<usize>,
}
impl HttpTransport for DefinitionTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse> {
        self.count.set(self.count.get() + 1);
        assert!(request.url.path().ends_with("/_apis/build/definitions"));
        assert!(
            request
                .url
                .query_pairs()
                .any(|(name, value)| name == "name" && value == "CI")
        );
        Ok(HttpResponse {
            status: 200,
            url: request.url.clone(),
            headers: HashMap::new(),
            body: serde_json::to_vec(&self.body).unwrap(),
        })
    }
}
fn definition_client(definitions: Value) -> AdoClient<DefinitionTransport> {
    AdoClient::with_transport(
        organization(),
        "fake-token",
        DefinitionTransport {
            body: json!({"value":definitions}),
            count: Cell::new(0),
        },
    )
}

#[test]
fn pipeline_name_resolution_requires_one_exact_case_insensitive_match() {
    let client = definition_client(json!([{"id":4,"name":"ci"}, {"id":5,"name":"CI other"}]));
    assert_eq!(
        resolve_definition(&client, "Example Project", "CI").unwrap(),
        4
    );
    assert_eq!(
        resolve_definition(&client, "Example Project", "123").unwrap(),
        123
    );
    assert_eq!(client.transport.count.get(), 1);
    assert!(resolve_definition(&client, "Example Project", "0").is_err());
    let client = definition_client(json!([]));
    assert!(
        resolve_definition(&client, "Example Project", "CI")
            .unwrap_err()
            .0
            .contains("No pipeline named")
    );
    let client = definition_client(json!([{"id":4,"name":"CI"},{"id":5,"name":"CI"}]));
    assert!(
        resolve_definition(&client, "Example Project", "CI")
            .unwrap_err()
            .0
            .contains("matches 2 definitions")
    );
}
