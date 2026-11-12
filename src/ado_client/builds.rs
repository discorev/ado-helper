use super::{AdoClient, HttpTransport, invalid_response, pagination};
use crate::{Error, Result};
use serde_json::Value;
use std::collections::HashSet;

pub struct BuildQuery<'a> {
    pub definition: Option<i64>,
    pub branch: Option<&'a str>,
    pub status: Option<&'a str>,
    pub result: Option<&'a str>,
    pub top: i64,
}

const TEST_PAGE_SIZE: usize = 100;

impl<T: HttpTransport> AdoClient<T> {
    pub fn build_definitions(&self, project: &str, name: Option<&str>) -> Result<Vec<Value>> {
        validate(project, 1)?;
        let mut query = vec![("includeAllProperties", "true")];
        if let Some(name) = name {
            query.push(("name", name));
        }
        self.build_pages(&[project, "_apis", "build", "definitions"], &query, None)
    }

    pub fn builds(&self, project: &str, filters: &BuildQuery<'_>) -> Result<Vec<Value>> {
        validate(project, filters.top)?;
        if filters.definition.is_some_and(|id| id <= 0) {
            return Err(Error("Pipeline definition ID must be positive.".into()));
        }
        let definition = filters.definition.map(|id| id.to_string());
        let mut query = vec![("queryOrder", "queueTimeDescending")];
        for (name, value) in [
            ("definitions", definition.as_deref()),
            ("branchName", filters.branch),
            ("statusFilter", filters.status),
            ("resultFilter", filters.result),
        ] {
            if let Some(value) = value {
                query.push((name, value));
            }
        }
        let top = usize::try_from(filters.top).map_err(|_| Error("--top is too large.".into()))?;
        self.build_pages(&[project, "_apis", "build", "builds"], &query, Some(top))
    }

    pub fn build(&self, project: &str, id: i64) -> Result<Value> {
        validate(project, id)?;
        self.get_object(
            &[project, "_apis", "build", "builds", &id.to_string()],
            &[],
            true,
        )
    }

    pub fn build_timeline(&self, project: &str, id: i64) -> Result<Vec<Value>> {
        validate(project, id)?;
        let object = self.get_object(
            &[
                project,
                "_apis",
                "build",
                "builds",
                &id.to_string(),
                "timeline",
            ],
            &[],
            true,
        )?;
        object
            .get("records")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(invalid_response)
    }

    pub fn build_log(&self, project: &str, id: i64, log_id: i64) -> Result<String> {
        validate(project, id)?;
        if log_id <= 0 {
            return Err(Error("Log ID must be positive.".into()));
        }
        let mut request = self.make_request(
            "GET",
            &[
                project,
                "_apis",
                "build",
                "builds",
                &id.to_string(),
                "logs",
                &log_id.to_string(),
            ],
            &[],
            true,
        )?;
        request.headers.insert("Accept".into(), "text/plain".into());
        let response = self.perform(&request)?;
        String::from_utf8(response.body)
            .map_err(|_| Error("Azure DevOps returned a log that is not UTF-8 text.".into()))
    }

    pub fn build_test_runs(&self, project: &str, id: i64) -> Result<Vec<Value>> {
        validate(project, id)?;
        let uri = format!("vstfs:///Build/Build/{id}");
        self.test_pages(&[project, "_apis", "test", "runs"], &[("buildUri", &uri)])
    }

    pub fn test_run_statistics(&self, project: &str, run_id: i64) -> Result<Value> {
        validate(project, run_id)?;
        self.get_object(
            &[
                project,
                "_apis",
                "test",
                "runs",
                &run_id.to_string(),
                "statistics",
            ],
            &[],
            true,
        )
    }

    pub fn test_results(&self, project: &str, run_id: i64, all: bool) -> Result<Vec<Value>> {
        validate(project, run_id)?;
        let query = if all {
            vec![]
        } else {
            vec![("outcomes", "Failed")]
        };
        let results = self.test_pages(
            &[
                project,
                "_apis",
                "test",
                "Runs",
                &run_id.to_string(),
                "results",
            ],
            &query,
        )?;
        // Keep the public contract even if the service returns an unfiltered page.
        Ok(results
            .into_iter()
            .filter(|result| {
                all || result
                    .get("outcome")
                    .and_then(Value::as_str)
                    .is_some_and(|outcome| outcome.eq_ignore_ascii_case("Failed"))
            })
            .collect())
    }

    fn build_pages(
        &self,
        path: &[&str],
        query: &[(&str, &str)],
        limit: Option<usize>,
    ) -> Result<Vec<Value>> {
        let mut output = Vec::new();
        let mut token: Option<String> = None;
        let mut seen = HashSet::new();
        loop {
            let mut query = query.to_vec();
            if let Some(token) = token.as_deref() {
                if token.is_empty() || !seen.insert(token.to_owned()) {
                    return Err(pagination());
                }
                query.push(("continuationToken", token));
            }
            let remaining = limit.map(|limit| limit.saturating_sub(output.len()).to_string());
            if let Some(remaining) = remaining.as_deref() {
                query.push(("$top", remaining));
            }
            let (object, response) = self.get_object_response(path, &query, true)?;
            output.extend(values(&object)?.iter().cloned());
            if let Some(limit) = limit {
                output.truncate(limit);
                if output.len() == limit {
                    break;
                }
            }
            token = response.headers.get("x-ms-continuationtoken").cloned();
            if token.is_none() {
                break;
            }
        }
        Ok(output)
    }

    fn test_pages(&self, path: &[&str], query: &[(&str, &str)]) -> Result<Vec<Value>> {
        let mut output = Vec::new();
        let mut skip: usize = 0;
        let mut previous = None;
        loop {
            let skip_value = skip.to_string();
            let top = TEST_PAGE_SIZE.to_string();
            let mut query = query.to_vec();
            query.extend([("$skip", skip_value.as_str()), ("$top", top.as_str())]);
            let object = self.get_object(path, &query, true)?;
            let page = values(&object)?;
            if !page.is_empty() && previous.as_ref() == Some(page) {
                return Err(pagination());
            }
            output.extend(page.iter().cloned());
            if page.len() < TEST_PAGE_SIZE {
                break;
            }
            skip = skip.checked_add(page.len()).ok_or_else(pagination)?;
            previous = Some(page.clone());
        }
        Ok(output)
    }
}

fn values(object: &Value) -> Result<&Vec<Value>> {
    object
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(invalid_response)
}

fn validate(project: &str, id: i64) -> Result<()> {
    if project.trim().is_empty() {
        Err(Error("Project must not be empty.".into()))
    } else if id <= 0 {
        Err(Error(
            "Build, test run and limit IDs must be positive.".into(),
        ))
    } else {
        Ok(())
    }
}
