use super::{Error, Organization, Result, Value, build_url, integer, json, positive_id};
use std::collections::{HashMap, HashSet};
use url::Url;

pub(super) fn definition_output(
    definition: &Value,
    organization: &Organization,
    project: &str,
) -> Result<Value> {
    let id = positive_id(definition)?;
    let mut url =
        Url::parse(&organization.url()).map_err(|_| Error("Invalid organization URL.".into()))?;
    url.path_segments_mut()
        .map_err(|_| Error("Invalid pipeline URL.".into()))?
        .extend([project, "_build"]);
    url.query_pairs_mut()
        .append_pair("definitionId", &id.to_string());
    Ok(json!({
        "id": id, "name": definition["name"], "folder": definition["path"],
        "yamlPath": definition.pointer("/process/yamlFilename"),
        "repository": definition.pointer("/repository/name"), "url": url.as_str(),
    }))
}

pub(super) fn run_output(
    build: &Value,
    organization: &Organization,
    project: &str,
) -> Result<Value> {
    let id = positive_id(build)?;
    Ok(json!({
        "id": id, "buildNumber": build["buildNumber"],
        "pipeline": {"id": build.pointer("/definition/id"), "name": build.pointer("/definition/name")},
        "status": build["status"], "result": build["result"],
        "sourceBranch": build["sourceBranch"], "sourceVersion": build["sourceVersion"],
        "reason": build["reason"], "requestedFor": build.pointer("/requestedFor/displayName"),
        "queueTime": build["queueTime"], "startTime": build["startTime"], "finishTime": build["finishTime"],
        "url": build_url(organization, project, id)?,
    }))
}

fn retained(record: &Value) -> bool {
    record
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| {
            ["Stage", "Job", "Task"]
                .iter()
                .any(|candidate| kind.eq_ignore_ascii_case(candidate))
        })
}

fn record_index(records: &[Value]) -> HashMap<&str, &Value> {
    records
        .iter()
        .filter_map(|record| {
            record
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id, record))
        })
        .collect()
}

/// Flatten intermediate Phase records while preserving Stage/Job/Task parent IDs.
pub(super) fn timeline_output(records: &[Value]) -> Result<Value> {
    let index = record_index(records);
    let output = records.iter().filter(|record| retained(record)).map(|record| {
        let mut parent_id = record.get("parentId").and_then(Value::as_str);
        let mut seen = HashSet::new();
        while let Some(id) = parent_id {
            if !seen.insert(id) { return Err(Error("Azure DevOps returned a cyclic timeline.".into())); }
            match index.get(id) {
                Some(parent) if !retained(parent) => parent_id = parent.get("parentId").and_then(Value::as_str),
                _ => break,
            }
        }
        let issues = record.get("issues").and_then(Value::as_array).into_iter().flatten()
            .filter(|issue| issue.get("type").and_then(Value::as_str).is_some_and(|kind| kind.eq_ignore_ascii_case("error") || kind.eq_ignore_ascii_case("warning")))
            .cloned().collect::<Vec<_>>();
        Ok(json!({
            "id": record["id"], "parentId": parent_id, "name": record["name"], "type": record["type"],
            "state": record["state"], "result": record["result"],
            "startTime": record["startTime"], "finishTime": record["finishTime"],
            "logId": record.pointer("/log/id"), "issues": issues,
        }))
    }).collect::<Result<Vec<_>>>()?;
    Ok(Value::Array(output))
}

/// Failed tasks as (log ID, "Stage / Job / Task") pairs. Tasks that failed before writing a log
/// (for example, during setup) have no log ID.
pub(super) fn failed_task_logs(records: &[Value]) -> Result<Vec<(Option<i64>, String)>> {
    let index = record_index(records);
    records
        .iter()
        .filter(|record| {
            record
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("Task"))
                && record
                    .get("result")
                    .and_then(Value::as_str)
                    .is_some_and(|result| result.eq_ignore_ascii_case("failed"))
        })
        .map(|record| {
            let log_id = record
                .pointer("/log/id")
                .and_then(integer)
                .filter(|id| *id > 0);
            let mut names = Vec::new();
            let mut current = Some(record);
            let mut seen = HashSet::new();
            while let Some(record) = current {
                if let Some(id) = record.get("id").and_then(Value::as_str)
                    && !seen.insert(id)
                {
                    return Err(Error("Azure DevOps returned a cyclic timeline.".into()));
                }
                if retained(record) {
                    names.push(
                        record
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("unnamed"),
                    );
                }
                current = record
                    .get("parentId")
                    .and_then(Value::as_str)
                    .and_then(|id| index.get(id).copied());
            }
            names.reverse();
            Ok((log_id, names.join(" / ")))
        })
        .collect()
}

/// Slice without normalising CRLFs or adding/removing a final newline.
pub(super) fn tail_lines(text: &str, tail: Option<usize>) -> &str {
    let Some(lines) = tail else {
        return text;
    };
    if lines == 0 {
        return "";
    }
    let mut end = text.len();
    if text.ends_with('\n') {
        end -= 1;
    }
    let start = text[..end]
        .rmatch_indices('\n')
        .nth(lines - 1)
        .map_or(0, |(position, _)| position + 1);
    &text[start..]
}

pub(super) fn test_run_output(run: &Value, statistics: &Value, results: &[Value]) -> Result<Value> {
    let id = positive_id(run)?;
    let counts = statistics
        .get("runStatistics")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            Error("Azure DevOps test statistics response is missing runStatistics.".into())
        })?;
    let count = |outcomes: &[&str]| -> i64 {
        counts
            .iter()
            .filter(|entry| {
                entry
                    .get("outcome")
                    .and_then(Value::as_str)
                    .is_some_and(|outcome| {
                        outcomes
                            .iter()
                            .any(|candidate| candidate.eq_ignore_ascii_case(outcome))
                    })
            })
            .filter_map(|entry| entry.get("count").and_then(integer))
            .sum()
    };
    let total = run.get("totalTests").and_then(integer).unwrap_or_else(|| {
        counts
            .iter()
            .filter_map(|entry| entry.get("count").and_then(integer))
            .sum()
    });
    let results = results.iter().map(|result| json!({
        "id": result["id"], "testName": result["testCaseTitle"], "automatedTestName": result["automatedTestName"],
        "outcome": result["outcome"], "errorMessage": result["errorMessage"], "stackTrace": result["stackTrace"],
        "durationInMs": result["durationInMs"], "testRun": {"id": id, "name": run["name"]},
    })).collect::<Vec<_>>();
    Ok(json!({
        "id": id, "name": run["name"], "state": run["state"], "total": total,
        "passed": count(&["Passed"]), "failed": count(&["Failed"]),
        "notExecuted": count(&["NotExecuted"]), "results": results,
    }))
}
