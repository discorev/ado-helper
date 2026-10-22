use super::{AdoClient, HttpTransport, integer, invalid_response, pagination};
use crate::{Error, Result};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

impl<T: HttpTransport> AdoClient<T> {
    pub fn wiql(&self, project: &str, query: &str, top: i64) -> Result<Vec<i64>> {
        self.validate_project(project)?;
        if query.trim().is_empty() {
            return Err(Error("WIQL query must not be empty.".into()));
        }
        if top <= 0 {
            return Err(Error("WIQL top must be greater than zero.".into()));
        }
        let top = top.to_string();
        let obj = self.send_object_options(
            "POST",
            &[project, "_apis", "wit", "wiql"],
            &[("$top", top.as_str())],
            &json!({"query": query}),
            Some("7.1"),
            "application/json",
        )?;
        obj.get("workItems")
            .and_then(Value::as_array)
            .ok_or_else(invalid_response)?
            .iter()
            .map(|item| {
                item.get("id")
                    .and_then(integer)
                    .filter(|id| *id > 0)
                    .ok_or_else(invalid_response)
            })
            .collect()
    }

    pub fn search_work_items(&self, project: &str, search: &str, top: i64) -> Result<Vec<i64>> {
        if search.is_empty() {
            return Err(Error("Work item search text must not be empty.".into()));
        }
        let escaped = search.replace('\'', "''");
        let query = format!(
            "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = @project AND ([System.Title] CONTAINS '{escaped}' OR [System.Description] CONTAINS '{escaped}') ORDER BY [System.ChangedDate] DESC"
        );
        self.wiql(project, &query, top)
    }

    pub fn work_items(&self, project: &str, ids: &[i64]) -> Result<Vec<Value>> {
        self.validate_project(project)?;
        if ids.iter().any(|id| *id <= 0) {
            return Err(Error("Work item IDs must be greater than zero.".into()));
        }
        let mut by_id = HashMap::new();
        for chunk in ids.chunks(200) {
            let joined = chunk
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let obj = self.get_object(
                &[project, "_apis", "wit", "workitems"],
                &[
                    ("ids", joined.as_str()),
                    (
                        "fields",
                        "System.Id,System.WorkItemType,System.Title,System.State,System.AssignedTo,System.AreaPath,System.IterationPath,System.Tags",
                    ),
                    // Items deleted or hidden since the WIQL ran come back as null instead of failing the batch.
                    ("errorPolicy", "omit"),
                ],
                true,
            )?;
            for item in obj
                .get("value")
                .and_then(Value::as_array)
                .ok_or_else(invalid_response)?
                .iter()
                .filter(|item| !item.is_null())
            {
                let id = item
                    .get("id")
                    .and_then(integer)
                    .filter(|id| *id > 0)
                    .ok_or_else(invalid_response)?;
                by_id.insert(id, item.clone());
            }
        }
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }

    pub fn work_item(&self, project: &str, id: i64) -> Result<Value> {
        self.validate_work_item(project, id)?;
        self.get_object(
            &[project, "_apis", "wit", "workitems", &id.to_string()],
            &[],
            true,
        )
    }

    pub fn work_item_comments(&self, project: &str, id: i64) -> Result<Vec<Value>> {
        self.validate_work_item(project, id)?;
        let path = [
            project,
            "_apis",
            "wit",
            "workItems",
            &id.to_string(),
            "comments",
        ];
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        let mut seen = HashSet::new();
        loop {
            if let Some(value) = &token
                && (value.is_empty() || !seen.insert(value.clone()))
            {
                return Err(pagination());
            }
            let query: Vec<(&str, &str)> = token
                .as_deref()
                .map(|value| vec![("continuationToken", value)])
                .unwrap_or_default();
            let (obj, response) =
                self.get_object_response_version(&path, &query, Some("7.1-preview.4"))?;
            out.extend(
                obj.get("comments")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid_response)?
                    .iter()
                    .cloned(),
            );
            token = obj
                .get("continuationToken")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .or_else(|| response.headers.get("x-ms-continuationtoken").cloned());
            if token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    pub fn add_work_item_comment(&self, project: &str, id: i64, text: &str) -> Result<Value> {
        self.validate_work_item(project, id)?;
        if text.trim().is_empty() {
            return Err(Error("Comment body must not be empty.".into()));
        }
        self.send_object_options(
            "POST",
            &[
                project,
                "_apis",
                "wit",
                "workItems",
                &id.to_string(),
                "comments",
            ],
            &[],
            &json!({"text": text}),
            Some("7.1-preview.4"),
            "application/json",
        )
    }

    pub fn create_work_item(
        &self,
        project: &str,
        work_item_type: &str,
        patch: &Value,
    ) -> Result<Value> {
        self.validate_project(project)?;
        if work_item_type.is_empty() {
            return Err(Error("Work item type must not be empty.".into()));
        }
        self.validate_patch(patch)?;
        let typed = format!("${work_item_type}");
        self.send_object_options(
            "POST",
            &[project, "_apis", "wit", "workitems", &typed],
            &[],
            patch,
            Some("7.1"),
            "application/json-patch+json",
        )
    }

    pub fn update_work_item(&self, project: &str, id: i64, patch: &Value) -> Result<Value> {
        self.validate_work_item(project, id)?;
        self.validate_patch(patch)?;
        self.send_object_options(
            "PATCH",
            &[project, "_apis", "wit", "workitems", &id.to_string()],
            &[],
            patch,
            Some("7.1"),
            "application/json-patch+json",
        )
    }

    fn validate_project(&self, project: &str) -> Result<()> {
        if project.is_empty() {
            Err(Error("Project must not be empty.".into()))
        } else {
            Ok(())
        }
    }
    fn validate_work_item(&self, project: &str, id: i64) -> Result<()> {
        self.validate_project(project)?;
        if id <= 0 {
            Err(Error("Work item ID must be greater than zero.".into()))
        } else {
            Ok(())
        }
    }
    fn validate_patch(&self, patch: &Value) -> Result<()> {
        if patch.as_array().is_none_or(Vec::is_empty) {
            Err(Error("Work item patch must not be empty.".into()))
        } else {
            Ok(())
        }
    }
}
