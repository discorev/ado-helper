use super::*;
use ado_core::ado_client::{HttpTransport, reviewer_entry};

impl Application {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pr_query(
        &self,
        status: &str,
        creator: Option<&str>,
        reviewer: Option<&str>,
        source: Option<&str>,
        target: Option<&str>,
        top: i64,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let remote = AzureGitRemote::discover(&GitRunner, None)?;
        let organization = Organization::parse(&remote.organization)?;
        let profile = self.selected_profile(profile_name, &organization)?;
        let client = AdoClient::new(organization.clone(), &self.keychain.read(&profile)?)?;
        let creator_id = filter_identity(&profile, creator);
        let reviewer_id = filter_identity(&profile, reviewer);
        let source_ref = source.map(branch_ref).transpose()?;
        let target_ref = target.map(branch_ref).transpose()?;
        let pull_requests = client.query_pull_requests(
            &remote.project,
            &remote.repository,
            status,
            creator_id.as_deref(),
            reviewer_id.as_deref(),
            source_ref.as_deref(),
            target_ref.as_deref(),
            top,
        )?;
        let output = pull_requests
            .iter()
            .map(|pull_request| {
                summarize_pull_request(
                    pull_request,
                    &organization,
                    &remote.project,
                    &remote.repository,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        write_json(&Value::Array(output))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pr_create(
        &self,
        source: Option<&str>,
        target: Option<&str>,
        title: &str,
        description_file: Option<&str>,
        draft: bool,
        reviewers: &[String],
        force: bool,
        work_items: &[i64],
        tags: Option<&str>,
        auto_complete: Option<&AutoCompleteOptions>,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let context = LocalGitContext::discover(&GitRunner, None)?;
        let organization = Organization::parse(&context.remote.organization)?;
        let profile = self.selected_profile(profile_name, &organization)?;
        let client = AdoClient::new(organization.clone(), &self.keychain.read(&profile)?)?;
        // Checked up front, before the pull request exists, so a mismatched token never leaves
        // an orphaned PR behind with auto-complete silently unset.
        let identity = if auto_complete.is_some() {
            let identity = client.identity()?;
            if identity.id != profile.identity.id {
                return Err(Error(format!(
                    "The saved token authenticated as a different account. No pull request was created; run 'ado auth update {}'.",
                    profile.name
                )));
            }
            Some(identity)
        } else {
            None
        };
        let source_name = source.unwrap_or(&context.branch);
        let source_ref = branch_ref(source_name)?;
        let source_tip = client.branch_tip(
            &context.remote.project,
            &context.remote.repository,
            &source_ref,
        )?;
        let current_ref = branch_ref(&context.branch)?;
        let local_head = if source_ref == current_ref {
            Some(
                GitRunner
                    .run(&["rev-parse".into(), "HEAD".into()], None)?
                    .to_ascii_lowercase(),
            )
        } else {
            None
        };
        validate_source_tip(
            source_name,
            &source_ref,
            &current_ref,
            local_head.as_deref(),
            source_tip.as_deref(),
        )?;
        let target_ref = if let Some(target) = target {
            branch_ref(target)?
        } else {
            let repository =
                client.repository(&context.remote.project, &context.remote.repository)?;
            repository
                .get("defaultBranch")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    Error(
                        "Azure DevOps did not return a default branch for the repository; pass --target BRANCH."
                            .into(),
                    )
                })
                .and_then(branch_ref)?
        };
        let description = description_file.map(read_body).transpose()?;
        let mut body = json!({
            "sourceRefName": source_ref,
            "targetRefName": target_ref,
            "title": title,
            "isDraft": draft,
            "workItemRefs": work_items.iter().map(|id| json!({"id": id.to_string()})).collect::<Vec<_>>(),
        });
        if let Some(description) = description {
            body["description"] = Value::String(description);
        }
        let created = client.create_pull_request(
            &context.remote.project,
            &context.remote.repository,
            &body,
        )?;
        let id = pull_request_id(&created)?;
        let mut added_reviewers = Vec::new();
        let follow_up: Result<()> = (|| {
            for reviewer in reviewers {
                let reviewer_id = client.add_pull_request_reviewer(
                    &context.remote.project,
                    &context.remote.repository,
                    id,
                    reviewer,
                    force,
                )?;
                added_reviewers.push((reviewer.clone(), reviewer_id));
            }
            for label in parse_labels(tags.unwrap_or("")) {
                client.add_pull_request_label(
                    &context.remote.project,
                    &context.remote.repository,
                    id,
                    &label,
                )?;
            }
            Ok(())
        })();
        let auto_complete_suffix = if auto_complete.is_some() {
            " Auto-complete was not set."
        } else {
            ""
        };
        if let Err(error) = follow_up {
            return Err(Error(format!(
                "Pull request {id} was created, but its reviewers or labels could not be completed: {error}{auto_complete_suffix}"
            )));
        }
        let final_pull_request = client.pull_request(id).map_err(|error| {
            Error(format!(
                "Pull request {id} was created, but final readback failed: {error}{auto_complete_suffix}"
            ))
        })?;
        if let Err(Error(error)) =
            validate_reviewers_present(&final_pull_request, &added_reviewers, "created")
        {
            return Err(Error(format!("{error}{auto_complete_suffix}")));
        }
        let Some(options) = auto_complete else {
            return write_json(&show_pull_request(
                &final_pull_request,
                &organization,
                &context.remote.project,
                &context.remote.repository,
            )?);
        };
        // Reviewers and labels are confirmed present before auto-complete is set, so a failed
        // follow-up never results in auto-complete being set on an incomplete pull request.
        let identity = identity.expect("auto_complete.is_some() implies identity was checked");
        match set_auto_complete(
            &client,
            &context.remote.project,
            &context.remote.repository,
            id,
            &final_pull_request,
            &identity.id,
            Some(options),
        ) {
            Ok(verified) => write_json(&show_pull_request(
                &verified,
                &organization,
                &context.remote.project,
                &context.remote.repository,
            )?),
            Err(error) => {
                // Print the created PR, but never let a print failure hide the auto-complete error.
                let _ = show_pull_request(
                    &final_pull_request,
                    &organization,
                    &context.remote.project,
                    &context.remote.repository,
                )
                .and_then(|shown| write_json(&shown));
                Err(Error(format!("Pull request {id} was created. {error}")))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pr_update(
        &self,
        target: PullRequestTarget,
        title: Option<&str>,
        description_file: Option<&str>,
        status: Option<&str>,
        draft: Option<bool>,
        add_reviewers: &[String],
        add_required_reviewers: &[String],
        force: bool,
        tags: Option<&str>,
        target_branch: Option<&str>,
        add_work_items: &[i64],
        profile_name: Option<&str>,
    ) -> Result<()> {
        let resolved = self.resolve(target, profile_name)?;
        let repository = repository_identity(&resolved.pull_request)?;
        let mut body = Map::new();
        // Azure rejects a retarget to the branch the PR already targets, so skip it.
        let retarget = match target_branch {
            Some(branch) => {
                let reference = branch_ref(branch)?;
                (resolved.pull_request["targetRefName"].as_str() != Some(reference.as_str()))
                    .then_some(reference)
            }
            None => None,
        };
        if let Some(reference) = &retarget {
            body.insert("targetRefName".into(), Value::String(reference.clone()));
        }
        if let Some(value) = title {
            body.insert("title".into(), Value::String(value.into()));
        }
        if let Some(path) = description_file {
            body.insert("description".into(), Value::String(read_body(path)?));
        }
        if let Some(value) = status {
            body.insert("status".into(), Value::String(value.into()));
        }
        if let Some(value) = draft {
            body.insert("isDraft".into(), Value::Bool(value));
        }
        if !body.is_empty() {
            resolved.client.update_pull_request(
                &repository.project,
                &repository.repository,
                resolved.locator.id,
                &Value::Object(body),
            )?;
        }
        if retarget.is_some() {
            // The preview merge is computed against the old target; queue a new one.
            resolved.client.update_pull_request(
                &repository.project,
                &repository.repository,
                resolved.locator.id,
                &json!({"mergeStatus": "queued"}),
            )?;
        }
        let mut added_reviewers = Vec::new();
        for reviewer in add_reviewers {
            let reviewer_id = resolved.client.add_pull_request_reviewer(
                &repository.project,
                &repository.repository,
                resolved.locator.id,
                reviewer,
                force,
            )?;
            added_reviewers.push((reviewer.clone(), reviewer_id));
        }
        let mut required_reviewers = Vec::new();
        for reviewer in add_required_reviewers {
            let (reviewer_id, vote) = resolved.client.require_pull_request_reviewer(
                &repository.project,
                &repository.repository,
                resolved.locator.id,
                reviewer,
            )?;
            required_reviewers.push((reviewer.clone(), reviewer_id, vote));
        }
        if let Some(tags) = tags {
            replace_labels(
                &resolved.client,
                &repository,
                resolved.locator.id,
                &parse_labels(tags),
            )?;
        }
        if !add_work_items.is_empty() {
            link_work_items(
                &resolved.client,
                &repository,
                &resolved.pull_request,
                resolved.locator.id,
                add_work_items,
            )?;
        }
        let updated = resolved.client.pull_request(resolved.locator.id)?;
        validate_reviewers_present(&updated, &added_reviewers, "updated")?;
        validate_required_reviewers(&updated, &required_reviewers)?;
        write_json(&show_pull_request(
            &updated,
            &resolved.locator.organization,
            &repository.project,
            &repository.repository,
        )?)
    }
}

fn branch_ref(value: &str) -> Result<String> {
    let reference = if value.starts_with("refs/heads/") {
        value.to_owned()
    } else {
        format!("refs/heads/{value}")
    };
    validate_ref(&reference).map_err(|_| Error(format!("Invalid branch name '{value}'.")))?;
    Ok(reference)
}

pub(super) fn validate_source_tip(
    source_name: &str,
    source_ref: &str,
    current_ref: &str,
    local_head: Option<&str>,
    remote_tip: Option<&str>,
) -> Result<()> {
    let remote_tip = remote_tip.ok_or_else(|| {
        Error(format!(
            "Source branch '{source_name}' does not exist on the Azure DevOps remote. Push it before creating a pull request."
        ))
    })?;
    if source_ref == current_ref {
        let local_head = local_head
            .filter(|head| is_commit_sha(head))
            .ok_or_else(|| Error("Git returned an invalid local HEAD commit.".into()))?;
        if !local_head.eq_ignore_ascii_case(remote_tip) {
            return Err(Error(
                "The remote source branch does not match local HEAD. Push the current branch before creating a pull request."
                    .into(),
            ));
        }
    }
    Ok(())
}

fn filter_identity(profile: &Profile, value: Option<&str>) -> Option<String> {
    value.map(|value| {
        if value.eq_ignore_ascii_case("me") {
            profile.identity.id.clone()
        } else {
            value.to_owned()
        }
    })
}

pub(super) fn parse_labels(value: &str) -> Vec<String> {
    let mut labels = BTreeMap::new();
    for label in value
        .split(',')
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        labels
            .entry(label.to_ascii_lowercase())
            .or_insert_with(|| label.to_owned());
    }
    labels.into_values().collect()
}

fn replace_labels(
    client: &AdoClient,
    repository: &RepositoryIdentity,
    id: i64,
    requested: &[String],
) -> Result<()> {
    let requested_by_name = requested
        .iter()
        .map(|label| (label.to_ascii_lowercase(), label))
        .collect::<BTreeMap<_, _>>();
    let existing = client.pull_request_labels(&repository.project, &repository.repository, id)?;
    let mut existing_names = BTreeSet::new();
    for label in existing {
        let name = label
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(invalid_pull_request_response)?;
        let normalized = name.to_ascii_lowercase();
        existing_names.insert(normalized.clone());
        if !requested_by_name.contains_key(&normalized) {
            let label_id = label
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(name);
            client.delete_pull_request_label(
                &repository.project,
                &repository.repository,
                id,
                label_id,
            )?;
        }
    }
    for (normalized, label) in requested_by_name {
        if !existing_names.contains(&normalized) {
            client.add_pull_request_label(
                &repository.project,
                &repository.repository,
                id,
                label,
            )?;
        }
    }
    Ok(())
}

/// Links each work item to the pull request from the work item side, because Azure DevOps
/// ignores `workItemRefs` on a pull request PATCH. Items that already carry the link are left
/// unchanged, since Azure returns 400 for a duplicate relation. The PR's linked work items are
/// read back afterwards to confirm every requested ID is present.
fn link_work_items<T: HttpTransport>(
    client: &AdoClient<T>,
    repository: &RepositoryIdentity,
    pull_request: &Value,
    id: i64,
    work_items: &[i64],
) -> Result<()> {
    let artifact = pull_request_artifact_url(pull_request, id)?;
    let mut linked: Vec<i64> = Vec::new();
    for work_item in work_items {
        let attempt = client
            .work_item_with_relations(&repository.project, *work_item)
            .and_then(|item| {
                if has_artifact_link(&item, &artifact) {
                    return Ok(());
                }
                client
                    .update_work_item(
                        &repository.project,
                        *work_item,
                        &json!([{
                            "op": "add",
                            "path": "/relations/-",
                            "value": {
                                "rel": "ArtifactLink",
                                "url": artifact,
                                "attributes": {"name": "Pull Request"},
                            },
                        }]),
                    )
                    .map(|_| ())
            });
        if let Err(Error(error)) = attempt {
            let already = if linked.is_empty() {
                " No work items were linked.".to_owned()
            } else {
                format!(" Already linked: {}.", join_ids(&linked))
            };
            return Err(Error(format!(
                "Pull request {id} was updated, but work item {work_item} could not be linked: {error}{already}"
            )));
        }
        linked.push(*work_item);
    }
    let present = client
        .pull_request_work_items(&repository.project, &repository.repository, id)
        .map_err(|Error(error)| {
            Error(format!(
                "Pull request {id} was updated, but the linked work items readback failed: {error}"
            ))
        })?;
    for work_item in work_items {
        if !present.contains(work_item) {
            return Err(Error(format!(
                "Pull request {id} was updated, but work item {work_item} is missing from the linked work items readback."
            )));
        }
    }
    Ok(())
}

/// The `vstfs:///Git/PullRequestId/{project}%2F{repository}%2F{id}` artifact URI Azure DevOps
/// uses for a pull request in work item relations. Both GUIDs come from the resolved PR.
fn pull_request_artifact_url(pull_request: &Value, id: i64) -> Result<String> {
    let guid = |pointer: &str| {
        pull_request
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };
    let (project_id, repository_id) = guid("/repository/project/id")
        .zip(guid("/repository/id"))
        .ok_or_else(|| {
            Error(
                "Azure DevOps pull request response is missing the project or repository ID needed to link work items."
                    .into(),
            )
        })?;
    Ok(format!(
        "vstfs:///Git/PullRequestId/{project_id}%2F{repository_id}%2F{id}"
    ))
}

/// Azure may return GUIDs in either case and `%2F` as `%2f`, so the match is case-insensitive.
fn has_artifact_link(work_item: &Value, artifact: &str) -> bool {
    work_item
        .get("relations")
        .and_then(Value::as_array)
        .is_some_and(|relations| {
            relations.iter().any(|relation| {
                relation.get("rel").and_then(Value::as_str) == Some("ArtifactLink")
                    && relation
                        .get("url")
                        .and_then(Value::as_str)
                        .is_some_and(|url| url.eq_ignore_ascii_case(artifact))
            })
        })
}

fn join_ids(ids: &[i64]) -> String {
    ids.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn validate_required_reviewers(
    pull_request: &Value,
    expected: &[(String, String, i32)],
) -> Result<()> {
    let id = pull_request_id(pull_request)?;
    for (user, reviewer_id, vote) in expected {
        let Some(reviewer) = reviewer_entry(pull_request, reviewer_id)
            .filter(|reviewer| reviewer.get("isRequired").and_then(Value::as_bool) == Some(true))
        else {
            return Err(Error(format!(
                "Pull request {id} was updated, but reviewer '{user}' is not shown as required in the readback."
            )));
        };
        let actual = reviewer.get("vote").and_then(Value::as_i64).unwrap_or(0) as i32;
        if actual != *vote {
            return Err(Error(format!(
                "Pull request {id} was updated, but reviewer '{user}' now shows vote {actual} instead of {vote}; check it with 'ado pr show'."
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_reviewers_present(
    pull_request: &Value,
    expected: &[(String, String)],
    action: &str,
) -> Result<()> {
    let id = pull_request_id(pull_request)?;
    for (user, reviewer_id) in expected {
        if reviewer_entry(pull_request, reviewer_id).is_none() {
            return Err(Error(format!(
                "Pull request {id} was {action}, but reviewer '{user}' is missing from the readback."
            )));
        }
    }
    Ok(())
}

fn pull_request_id(value: &Value) -> Result<i64> {
    value
        .get("pullRequestId")
        .and_then(integer)
        .filter(|id| *id > 0)
        .ok_or_else(invalid_pull_request_response)
}

fn summarize_pull_request(
    pull_request: &Value,
    organization: &Organization,
    project: &str,
    repository: &str,
) -> Result<Value> {
    let id = pull_request_id(pull_request)?;
    let reviewers = pull_request
        .get("reviewers")
        .and_then(Value::as_array)
        .map(|reviewers| {
            reviewers
                .iter()
                .map(|reviewer| {
                    json!({
                        "identity": identity_name(reviewer),
                        "vote": reviewer.get("vote").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(json!({
        "id": id,
        "title": pull_request.get("title").cloned().unwrap_or(Value::Null),
        "status": pull_request.get("status").cloned().unwrap_or(Value::Null),
        "isDraft": pull_request.get("isDraft").cloned().unwrap_or(Value::Bool(false)),
        "creator": identity_name(pull_request.get("createdBy").unwrap_or(&Value::Null)),
        "source": pull_request.get("sourceRefName").cloned().unwrap_or(Value::Null),
        "target": pull_request.get("targetRefName").cloned().unwrap_or(Value::Null),
        "reviewers": reviewers,
        "url": pull_request_url(organization, project, repository, id)?,
    }))
}

fn show_pull_request(
    pull_request: &Value,
    organization: &Organization,
    project: &str,
    repository: &str,
) -> Result<Value> {
    let id = pull_request_id(pull_request)?;
    Ok(json!({
        "id": id,
        "url": pull_request_url(organization, project, repository, id)?,
        "fields": pull_request,
    }))
}

fn identity_name(value: &Value) -> Value {
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

pub(super) fn pull_request_url(
    organization: &Organization,
    project: &str,
    repository: &str,
    id: i64,
) -> Result<String> {
    let mut url = Url::parse(&organization.url())
        .map_err(|_| Error("Could not construct the pull request URL.".into()))?;
    url.path_segments_mut()
        .map_err(|_| Error("Could not construct the pull request URL.".into()))?
        .extend([project, "_git", repository, "pullrequest", &id.to_string()]);
    Ok(url.into())
}

fn invalid_pull_request_response() -> Error {
    Error("Azure DevOps returned a pull request response in an unexpected format.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ado_core::ado_client::{HttpRequest, HttpResponse};
    use std::sync::Mutex;

    const ARTIFACT: &str = "vstfs:///Git/PullRequestId/PROJ-GUID%2FREPO-GUID%2F42";

    /// Replays canned responses in order and records every request it sees.
    struct Script {
        responses: Mutex<Vec<(u16, Value)>>,
        requests: Mutex<Vec<HttpRequest>>,
    }

    impl Script {
        fn new(responses: Vec<(u16, Value)>) -> Self {
            Self {
                responses: Mutex::new(responses),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl HttpTransport for Script {
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse> {
            self.requests.lock().unwrap().push(request.clone());
            let mut responses = self.responses.lock().unwrap();
            assert!(!responses.is_empty(), "unexpected request {}", request.url);
            let (status, body) = responses.remove(0);
            Ok(HttpResponse {
                status,
                url: request.url.clone(),
                headers: Default::default(),
                body: serde_json::to_vec(&body).unwrap(),
            })
        }
    }

    fn scripted(script: Script) -> AdoClient<Script> {
        AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", script)
    }

    fn repository() -> RepositoryIdentity {
        RepositoryIdentity {
            project: "Project".into(),
            repository: "Repo".into(),
            project_aliases: vec![],
            repository_aliases: vec![],
        }
    }

    fn pull_request() -> Value {
        json!({"pullRequestId": 42, "repository": {"id": "REPO-GUID", "name": "Repo", "project": {"id": "PROJ-GUID", "name": "Project"}}})
    }

    fn requests(client: &AdoClient<Script>) -> Vec<(String, String, Option<Value>)> {
        client
            .transport
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| {
                (
                    request.method.clone(),
                    format!(
                        "{}?{}",
                        request.url.path(),
                        request.url.query().unwrap_or("")
                    ),
                    request
                        .body
                        .as_ref()
                        .map(|body| serde_json::from_slice(body).unwrap()),
                )
            })
            .collect()
    }

    #[test]
    fn link_work_items_skips_existing_links_patches_the_rest_and_reads_back() {
        let client = scripted(Script::new(vec![
            // 12 already carries the link, in Azure's lowercase form.
            (
                200,
                json!({"id": 12, "relations": [{"rel": "ArtifactLink", "url": ARTIFACT.to_ascii_lowercase()}]}),
            ),
            // 13 has an unrelated relation only.
            (
                200,
                json!({"id": 13, "relations": [{"rel": "System.LinkTypes.Hierarchy-Reverse", "url": "https://dev.azure.com/acme/_apis/wit/workItems/1"}]}),
            ),
            (200, json!({"id": 13, "relations": []})),
            (200, json!({"value": [{"id": "12"}, {"id": "13"}]})),
        ]));
        link_work_items(&client, &repository(), &pull_request(), 42, &[12, 13]).unwrap();
        let seen = requests(&client);
        assert_eq!(seen.len(), 4);
        assert_eq!(
            (seen[0].0.as_str(), seen[0].1.as_str()),
            (
                "GET",
                "/acme/Project/_apis/wit/workitems/12?%24expand=relations&api-version=7.1"
            )
        );
        assert_eq!(seen[1].0, "GET");
        assert_eq!(seen[2].0, "PATCH");
        assert_eq!(
            seen[2].1,
            "/acme/Project/_apis/wit/workitems/13?api-version=7.1"
        );
        assert_eq!(
            seen[2].2,
            Some(json!([{
                "op": "add",
                "path": "/relations/-",
                "value": {"rel": "ArtifactLink", "url": ARTIFACT, "attributes": {"name": "Pull Request"}}
            }]))
        );
        assert_eq!(
            (seen[3].0.as_str(), seen[3].1.as_str()),
            (
                "GET",
                "/acme/Project/_apis/git/repositories/Repo/pullrequests/42/workitems?api-version=7.1"
            )
        );
    }

    #[test]
    fn link_work_items_reports_already_linked_ids_on_partial_failure() {
        let client = scripted(Script::new(vec![
            (200, json!({"id": 12, "relations": []})),
            (200, json!({"id": 12})),
            (200, json!({"id": 13, "relations": []})),
            (400, json!({"message": "Relation already exists."})),
        ]));
        let error = link_work_items(&client, &repository(), &pull_request(), 42, &[12, 13])
            .unwrap_err()
            .to_string();
        assert!(
            error
                .starts_with("Pull request 42 was updated, but work item 13 could not be linked: "),
            "{error}"
        );
        assert!(error.contains("Relation already exists."), "{error}");
        assert!(error.ends_with(" Already linked: 12."), "{error}");

        let client = scripted(Script::new(vec![(404, json!({"message": "Not found."}))]));
        let error = link_work_items(&client, &repository(), &pull_request(), 42, &[12])
            .unwrap_err()
            .to_string();
        assert!(error.ends_with(" No work items were linked."), "{error}");
    }

    #[test]
    fn link_work_items_fails_when_readback_omits_a_requested_id() {
        let client = scripted(Script::new(vec![
            (200, json!({"id": 12, "relations": []})),
            (200, json!({"id": 12})),
            (200, json!({"value": [{"id": "7"}]})),
        ]));
        assert_eq!(
            link_work_items(&client, &repository(), &pull_request(), 42, &[12])
                .unwrap_err()
                .to_string(),
            "Pull request 42 was updated, but work item 12 is missing from the linked work items readback."
        );
    }

    #[test]
    fn artifact_url_requires_project_and_repository_guids() {
        assert_eq!(
            pull_request_artifact_url(&pull_request(), 42).unwrap(),
            ARTIFACT
        );
        for pointer in ["/repository/project/id", "/repository/id"] {
            let mut pull_request = pull_request();
            *pull_request.pointer_mut(pointer).unwrap() = Value::Null;
            let client = scripted(Script::new(vec![]));
            assert_eq!(
                link_work_items(&client, &repository(), &pull_request, 42, &[12])
                    .unwrap_err()
                    .to_string(),
                "Azure DevOps pull request response is missing the project or repository ID needed to link work items."
            );
            assert!(client.transport.requests.lock().unwrap().is_empty());
        }
    }
}
