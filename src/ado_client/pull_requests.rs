use super::{AdoClient, HttpTransport, invalid_response, pagination};
use crate::{Error, Result, git::validate_ref};
use serde_json::{Value, json};
use std::collections::HashSet;

impl<T: HttpTransport> AdoClient<T> {
    pub fn repository(&self, project: &str, repository: &str) -> Result<Value> {
        self.validate_resource(project, repository, 1)?;
        self.get_object(
            &[project, "_apis", "git", "repositories", repository],
            &[],
            true,
        )
    }

    pub fn branch_tip(
        &self,
        project: &str,
        repository: &str,
        reference: &str,
    ) -> Result<Option<String>> {
        self.validate_resource(project, repository, 1)?;
        validate_ref(reference)?;
        let filter = reference.strip_prefix("refs/").unwrap_or(reference);
        let object = self.get_object(
            &[project, "_apis", "git", "repositories", repository, "refs"],
            &[("filter", filter)],
            true,
        )?;
        let values = object
            .get("value")
            .and_then(Value::as_array)
            .ok_or_else(invalid_response)?;
        let matching = values
            .iter()
            .filter(|value| value.get("name").and_then(Value::as_str) == Some(reference))
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(invalid_response());
        }
        matching
            .first()
            .map(|value| {
                value
                    .get("objectId")
                    .and_then(Value::as_str)
                    .filter(|id| crate::git::is_commit_sha(id))
                    .map(|id| id.to_ascii_lowercase())
                    .ok_or_else(invalid_response)
            })
            .transpose()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn query_pull_requests(
        &self,
        project: &str,
        repository: &str,
        status: &str,
        creator: Option<&str>,
        reviewer: Option<&str>,
        source_ref: Option<&str>,
        target_ref: Option<&str>,
        top: i64,
    ) -> Result<Vec<Value>> {
        self.validate_resource(project, repository, 1)?;
        if !["active", "completed", "abandoned", "all"].contains(&status) {
            return Err(Error("Pull request status is invalid.".into()));
        }
        if top <= 0 {
            return Err(Error("Pull request top must be greater than zero.".into()));
        }
        if creator.is_some_and(str::is_empty) || reviewer.is_some_and(str::is_empty) {
            return Err(Error("Pull request identity must not be empty.".into()));
        }
        if let Some(reference) = source_ref {
            validate_ref(reference)?;
        }
        if let Some(reference) = target_ref {
            validate_ref(reference)?;
        }

        let mut results = Vec::new();
        let mut skip = 0_i64;
        let mut seen = HashSet::new();
        while results.len() < top as usize {
            let remaining = top - results.len() as i64;
            let page_size = remaining.min(100);
            let top_value = page_size.to_string();
            let skip_value = skip.to_string();
            let mut query = vec![
                ("$top", top_value.as_str()),
                ("$skip", skip_value.as_str()),
                ("searchCriteria.status", status),
            ];
            if let Some(value) = source_ref {
                query.push(("searchCriteria.sourceRefName", value));
            }
            if let Some(value) = target_ref {
                query.push(("searchCriteria.targetRefName", value));
            }
            let object = self.get_object(
                &[
                    project,
                    "_apis",
                    "git",
                    "repositories",
                    repository,
                    "pullrequests",
                ],
                &query,
                true,
            )?;
            let page = object
                .get("value")
                .and_then(Value::as_array)
                .ok_or_else(invalid_response)?;
            results.extend(
                page.iter()
                    .filter(|pull_request| pull_request_matches(pull_request, creator, reviewer))
                    .cloned(),
            );
            if page.len() < page_size as usize {
                break;
            }
            let fingerprint = serde_json::to_vec(page).map_err(|_| invalid_response())?;
            if !seen.insert(fingerprint) {
                return Err(pagination());
            }
            skip = skip.checked_add(page_size).ok_or_else(pagination)?;
        }
        results.truncate(top as usize);
        Ok(results)
    }

    pub fn create_pull_request(
        &self,
        project: &str,
        repository: &str,
        body: &Value,
    ) -> Result<Value> {
        self.validate_resource(project, repository, 1)?;
        if !body.is_object() {
            return Err(Error("Pull request body must be an object.".into()));
        }
        self.send_object(
            "POST",
            &[
                project,
                "_apis",
                "git",
                "repositories",
                repository,
                "pullrequests",
            ],
            body,
        )
    }

    pub fn update_pull_request(
        &self,
        project: &str,
        repository: &str,
        id: i64,
        body: &Value,
    ) -> Result<Value> {
        self.validate_resource(project, repository, id)?;
        if body.as_object().is_none_or(serde_json::Map::is_empty) {
            return Err(Error("Pull request update must not be empty.".into()));
        }
        self.send_object(
            "PATCH",
            &[
                project,
                "_apis",
                "git",
                "repositories",
                repository,
                "pullrequests",
                &id.to_string(),
            ],
            body,
        )
    }

    pub fn add_pull_request_reviewer(
        &self,
        project: &str,
        repository: &str,
        id: i64,
        reviewer: &str,
    ) -> Result<Value> {
        self.validate_resource(project, repository, id)?;
        if reviewer.trim().is_empty() {
            return Err(Error("Reviewer identity must not be empty.".into()));
        }
        self.send_object(
            "PUT",
            &[
                project,
                "_apis",
                "git",
                "repositories",
                repository,
                "pullrequests",
                &id.to_string(),
                "reviewers",
            ],
            &json!({"uniqueName": reviewer, "vote": 0}),
        )
    }

    pub fn pull_request_labels(
        &self,
        project: &str,
        repository: &str,
        id: i64,
    ) -> Result<Vec<Value>> {
        self.validate_resource(project, repository, id)?;
        let object = self.get_object(
            &[
                project,
                "_apis",
                "git",
                "repositories",
                repository,
                "pullrequests",
                &id.to_string(),
                "labels",
            ],
            &[],
            true,
        )?;
        object
            .get("value")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(invalid_response)
    }

    pub fn add_pull_request_label(
        &self,
        project: &str,
        repository: &str,
        id: i64,
        label: &str,
    ) -> Result<Value> {
        self.validate_resource(project, repository, id)?;
        if label.is_empty() {
            return Err(Error("Pull request label must not be empty.".into()));
        }
        self.send_object(
            "POST",
            &[
                project,
                "_apis",
                "git",
                "repositories",
                repository,
                "pullrequests",
                &id.to_string(),
                "labels",
            ],
            &json!({"name": label}),
        )
    }

    pub fn delete_pull_request_label(
        &self,
        project: &str,
        repository: &str,
        id: i64,
        label_id: &str,
    ) -> Result<()> {
        self.validate_resource(project, repository, id)?;
        if label_id.is_empty() {
            return Err(Error("Pull request label ID must not be empty.".into()));
        }
        self.send_empty(
            "DELETE",
            &[
                project,
                "_apis",
                "git",
                "repositories",
                repository,
                "pullrequests",
                &id.to_string(),
                "labels",
                label_id,
            ],
            &[],
            Some("7.1"),
        )
    }
}

fn pull_request_matches(
    pull_request: &Value,
    creator: Option<&str>,
    reviewer: Option<&str>,
) -> bool {
    creator.is_none_or(|expected| {
        pull_request
            .get("createdBy")
            .is_some_and(|identity| identity_matches(identity, expected))
    }) && reviewer.is_none_or(|expected| {
        pull_request
            .get("reviewers")
            .and_then(Value::as_array)
            .is_some_and(|reviewers| {
                reviewers
                    .iter()
                    .any(|identity| identity_matches(identity, expected))
            })
    })
}

fn identity_matches(identity: &Value, expected: &str) -> bool {
    ["id", "uniqueName", "displayName"]
        .into_iter()
        .filter_map(|name| identity.get(name).and_then(Value::as_str))
        .any(|value| value.eq_ignore_ascii_case(expected))
}
