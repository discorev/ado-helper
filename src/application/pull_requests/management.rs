use super::*;

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
        work_items: &[i64],
        tags: Option<&str>,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let context = LocalGitContext::discover(&GitRunner, None)?;
        let organization = Organization::parse(&context.remote.organization)?;
        let profile = self.selected_profile(profile_name, &organization)?;
        let client = AdoClient::new(organization.clone(), &self.keychain.read(&profile)?)?;
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
        let follow_up: Result<()> = (|| {
            for reviewer in reviewers {
                client.add_pull_request_reviewer(
                    &context.remote.project,
                    &context.remote.repository,
                    id,
                    reviewer,
                )?;
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
        if let Err(error) = follow_up {
            return Err(Error(format!(
                "Pull request {id} was created, but its reviewers or labels could not be completed: {error}"
            )));
        }
        let final_pull_request = client.pull_request(id).map_err(|error| {
            Error(format!(
                "Pull request {id} was created, but final readback failed: {error}"
            ))
        })?;
        write_json(&show_pull_request(
            &final_pull_request,
            &organization,
            &context.remote.project,
            &context.remote.repository,
        )?)
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
        tags: Option<&str>,
        profile_name: Option<&str>,
    ) -> Result<()> {
        let resolved = self.resolve(target, profile_name)?;
        let repository = repository_identity(&resolved.pull_request)?;
        let mut body = Map::new();
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
        for reviewer in add_reviewers {
            resolved.client.add_pull_request_reviewer(
                &repository.project,
                &repository.repository,
                resolved.locator.id,
                reviewer,
            )?;
        }
        if let Some(tags) = tags {
            replace_labels(
                &resolved.client,
                &repository,
                resolved.locator.id,
                &parse_labels(tags),
            )?;
        }
        let updated = resolved.client.pull_request(resolved.locator.id)?;
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
