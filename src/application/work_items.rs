use super::{
    Application,
    output::{integer, open_browser, read_body, write_json},
};
use ado_core::{
    Error, Result,
    ado_client::AdoClient,
    cli::WorkItemQuery,
    git::{AzureGitRemote, GitRunner},
    keychain::CredentialStore,
    organization::Organization,
    profiles::Profile,
};
use serde_json::{Map, Value, json};
use url::Url;

struct Scope {
    organization: Organization,
    project: String,
    profile: Profile,
}

impl Application {
    pub(super) fn wi_query(
        &self,
        query: WorkItemQuery,
        top: i64,
        profile_name: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.work_item_scope(profile_name, project)?;
        let client = self.work_item_client(&scope)?;
        let ids = match query {
            WorkItemQuery::Wiql(query) => client.wiql(&scope.project, &query, top)?,
            WorkItemQuery::WiqlFile(path) => {
                client.wiql(&scope.project, &read_body(&path)?, top)?
            }
            WorkItemQuery::Search(search) => {
                client.search_work_items(&scope.project, &search, top)?
            }
        };
        let items = client.work_items(&scope.project, &ids)?;
        let output = items
            .iter()
            .map(|item| summarize_work_item(item, &scope.organization, &scope.project))
            .collect::<Result<Vec<_>>>()?;
        write_json(&Value::Array(output))
    }

    pub(super) fn wi_show(
        &self,
        id: i64,
        profile_name: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.work_item_scope(profile_name, project)?;
        let item = self
            .work_item_client(&scope)?
            .work_item(&scope.project, id)?;
        write_json(&show_work_item(&item, &scope.organization, &scope.project)?)
    }

    pub(super) fn wi_comments(
        &self,
        id: i64,
        profile_name: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.work_item_scope(profile_name, project)?;
        let comments = self
            .work_item_client(&scope)?
            .work_item_comments(&scope.project, id)?
            .iter()
            .map(comment_output)
            .collect::<Result<Vec<_>>>()?;
        write_json(&Value::Array(comments))
    }

    pub(super) fn wi_comment(
        &self,
        id: i64,
        body_file: &str,
        profile_name: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.work_item_scope(profile_name, project)?;
        let comment = self.work_item_client(&scope)?.add_work_item_comment(
            &scope.project,
            id,
            &read_body(body_file)?,
        )?;
        write_json(&comment_output(&comment)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn wi_create(
        &self,
        work_item_type: &str,
        title: &str,
        description_file: Option<&str>,
        assign_to: Option<&str>,
        tags: Option<&str>,
        profile_name: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.work_item_scope(profile_name, project)?;
        let mut patch = Vec::new();
        add_field(&mut patch, "System.Title", title);
        if let Some(path) = description_file {
            add_field(&mut patch, "System.Description", &read_body(path)?);
        }
        if let Some(value) = assign_to {
            add_field(&mut patch, "System.AssignedTo", value);
        }
        if let Some(value) = tags {
            add_field(&mut patch, "System.Tags", value);
        }
        let item = self.work_item_client(&scope)?.create_work_item(
            &scope.project,
            work_item_type,
            &Value::Array(patch),
        )?;
        write_json(&show_work_item(&item, &scope.organization, &scope.project)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn wi_update(
        &self,
        id: i64,
        state: Option<&str>,
        title: Option<&str>,
        assign_to: Option<&str>,
        tags: Option<&str>,
        description_file: Option<&str>,
        profile_name: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.work_item_scope(profile_name, project)?;
        let mut patch = Vec::new();
        if let Some(value) = state {
            add_field(&mut patch, "System.State", value);
        }
        if let Some(value) = title {
            add_field(&mut patch, "System.Title", value);
        }
        if let Some(value) = assign_to {
            add_field(&mut patch, "System.AssignedTo", value);
        }
        if let Some(value) = tags {
            add_field(&mut patch, "System.Tags", value);
        }
        if let Some(path) = description_file {
            add_field(&mut patch, "System.Description", &read_body(path)?);
        }
        let item = self.work_item_client(&scope)?.update_work_item(
            &scope.project,
            id,
            &Value::Array(patch),
        )?;
        write_json(&show_work_item(&item, &scope.organization, &scope.project)?)
    }

    pub(super) fn wi_open(
        &self,
        id: i64,
        profile_name: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.work_item_scope(profile_name, project)?;
        let url = work_item_url(&scope.organization, &scope.project, id)?;
        println!("{url}");
        open_browser(&url)
    }

    fn work_item_scope(&self, profile_name: Option<&str>, project: Option<&str>) -> Result<Scope> {
        if project.is_some_and(|value| value.is_empty()) {
            return Err(Error("--project must not be empty.".into()));
        }
        let remote = if profile_name.is_none() || project.is_none() {
            Some(AzureGitRemote::discover(&GitRunner, None).map_err(|_| {
                if project.is_none() {
                    Error(
                        "Could not derive the Azure DevOps project from the current Git remote; pass --project NAME."
                            .into(),
                    )
                } else {
                    Error(
                        "Could not derive the Azure DevOps organization from the current Git remote; pass --profile NAME."
                            .into(),
                    )
                }
            })?)
        } else {
            None
        };
        let profile = if let Some(name) = profile_name {
            let profile = self.store.profile_by_name(name)?;
            if remote.as_ref().is_some_and(|remote| {
                !profile
                    .organization
                    .name
                    .eq_ignore_ascii_case(&remote.organization)
            }) {
                return Err(Error(format!(
                    "Profile '{}' belongs to {}, not {}.",
                    profile.name,
                    profile.organization.name,
                    remote.as_ref().unwrap().organization
                )));
            }
            profile
        } else {
            let organization = Organization::parse(&remote.as_ref().unwrap().organization)?;
            self.store.profile_by_organization(&organization)?
        };
        let project = project
            .map(str::to_owned)
            .or_else(|| remote.map(|remote| remote.project))
            .ok_or_else(|| Error("Pass --project NAME.".into()))?;
        Ok(Scope {
            organization: profile.organization.clone(),
            project,
            profile,
        })
    }

    fn work_item_client(&self, scope: &Scope) -> Result<AdoClient> {
        AdoClient::new(
            scope.organization.clone(),
            &self.keychain.read(&scope.profile)?,
        )
    }
}

fn add_field(patch: &mut Vec<Value>, field: &str, value: &str) {
    patch.push(json!({
        "op": "add",
        "path": format!("/fields/{field}"),
        "value": value,
    }));
}

fn summarize_work_item(item: &Value, organization: &Organization, project: &str) -> Result<Value> {
    let id = work_item_id(item)?;
    let fields = fields(item)?;
    Ok(json!({
        "id": id,
        "type": field(fields, "System.WorkItemType"),
        "title": field(fields, "System.Title"),
        "state": field(fields, "System.State"),
        "assignedTo": identity(field(fields, "System.AssignedTo")),
        "areaPath": field(fields, "System.AreaPath"),
        "iterationPath": field(fields, "System.IterationPath"),
        "tags": tags(field(fields, "System.Tags")),
        "url": work_item_url(organization, project, id)?,
    }))
}

fn show_work_item(item: &Value, organization: &Organization, project: &str) -> Result<Value> {
    let id = work_item_id(item)?;
    Ok(json!({
        "id": id,
        "url": work_item_url(organization, project, id)?,
        "fields": fields(item)?,
    }))
}

fn comment_output(comment: &Value) -> Result<Value> {
    let object = comment.as_object().ok_or_else(invalid_work_item_response)?;
    let id = object
        .get("id")
        .and_then(integer)
        .filter(|id| *id > 0)
        .ok_or_else(invalid_work_item_response)?;
    Ok(json!({
        "id": id,
        "author": identity(object.get("createdBy").unwrap_or(&Value::Null)),
        "createdDate": object.get("createdDate").cloned().unwrap_or(Value::Null),
        "text": object.get("text").cloned().unwrap_or(Value::Null),
    }))
}

fn work_item_id(item: &Value) -> Result<i64> {
    item.get("id")
        .and_then(integer)
        .filter(|id| *id > 0)
        .ok_or_else(invalid_work_item_response)
}

fn fields(item: &Value) -> Result<&Map<String, Value>> {
    item.get("fields")
        .and_then(Value::as_object)
        .ok_or_else(invalid_work_item_response)
}

fn field<'a>(fields: &'a Map<String, Value>, name: &str) -> &'a Value {
    fields.get(name).unwrap_or(&Value::Null)
}

fn identity(value: &Value) -> Value {
    value
        .get("uniqueName")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            value
                .get("displayName")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .map_or(Value::Null, |value| Value::String(value.to_owned()))
}

fn tags(value: &Value) -> Value {
    Value::Array(
        value
            .as_str()
            .into_iter()
            .flat_map(|value| value.split(';'))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Value::String(value.to_owned()))
            .collect(),
    )
}

fn work_item_url(organization: &Organization, project: &str, id: i64) -> Result<String> {
    let mut url = Url::parse(&organization.url()).map_err(|_| invalid_work_item_response())?;
    url.path_segments_mut()
        .map_err(|_| invalid_work_item_response())?
        .extend([project, "_workitems", "edit", &id.to_string()]);
    Ok(url.into())
}

fn invalid_work_item_response() -> Error {
    Error("Azure DevOps returned a work item response in an unexpected format.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_normalizes_identity_tags_and_url() {
        let item = json!({
            "id": 42,
            "fields": {
                "System.WorkItemType": "Bug",
                "System.Title": "Broken login",
                "System.State": "Active",
                "System.AssignedTo": {"displayName": "Ada", "uniqueName": "ada@example.test"},
                "System.AreaPath": "Web",
                "System.IterationPath": "Web\\Sprint 1",
                "System.Tags": " auth ; urgent; ;",
            }
        });
        let output =
            summarize_work_item(&item, &Organization::parse("acme").unwrap(), "My Project")
                .unwrap();
        assert_eq!(output["assignedTo"], "ada@example.test");
        assert_eq!(output["tags"], json!(["auth", "urgent"]));
        assert_eq!(
            output["url"],
            "https://dev.azure.com/acme/My%20Project/_workitems/edit/42"
        );
    }

    #[test]
    fn comment_author_falls_back_to_display_name() {
        assert_eq!(
            comment_output(&json!({
                "id": 7,
                "createdBy": {"displayName": "Ada"},
                "createdDate": "2026-01-01T00:00:00Z",
                "text": "Hello",
            }))
            .unwrap(),
            json!({
                "id": 7,
                "author": "Ada",
                "createdDate": "2026-01-01T00:00:00Z",
                "text": "Hello",
            })
        );
    }
}
