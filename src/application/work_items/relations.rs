use super::*;

/// `wi create` has no existing relations to check against, so the link ops are just appended to
/// the rest of the field patch before creating the item.
pub(super) fn apply_create<T: HttpTransport>(
    client: &AdoClient<T>,
    organization: &Organization,
    project: &str,
    work_item_type: &str,
    mut patch: Vec<Value>,
    links: &[WorkItemLink],
) -> Result<Value> {
    patch.extend(links.iter().map(|link| add_op(organization, link)));
    client.create_work_item(project, work_item_type, &Value::Array(patch))
}

/// Applies `links`/`remove_links` (plus any other field ops already in `patch`) to work item
/// `id`. Existing relations are fetched first only when there is a link option to check against;
/// that same read is returned as the result when filtering leaves nothing left to change, so
/// `wi update --link` with everything already linked costs a single GET.
pub(super) fn apply_update<T: HttpTransport>(
    client: &AdoClient<T>,
    organization: &Organization,
    project: &str,
    id: i64,
    mut patch: Vec<Value>,
    links: &[WorkItemLink],
    remove_links: &[WorkItemLink],
) -> Result<Value> {
    let current = if links.is_empty() && remove_links.is_empty() {
        None
    } else {
        let current = client.work_item_with_relations(project, id)?;
        patch.extend(update_ops(id, &current, organization, links, remove_links)?);
        Some(current)
    };
    if patch.is_empty() {
        match current {
            Some(current) => Ok(current),
            None => client.work_item_with_relations(project, id),
        }
    } else {
        client.update_work_item(project, id, &Value::Array(patch))
    }
}

/// Builds the patch ops needed to apply `links`/`remove_links` to `item` (fetched with
/// relations). Adds that already exist are skipped; a remove whose link does not exist is an
/// error. Removes are emitted first, in descending index order, so removing one never shifts
/// the index of another still to be removed.
fn update_ops(
    id: i64,
    item: &Value,
    organization: &Organization,
    links: &[WorkItemLink],
    remove_links: &[WorkItemLink],
) -> Result<Vec<Value>> {
    let existing = item
        .get("relations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut remove_indexes = Vec::new();
    for link in remove_links {
        let index = existing
            .iter()
            .position(|relation| matches_link(relation, link))
            .ok_or_else(|| {
                Error(format!(
                    "Work item {id} has no {} link to {}.",
                    link.link_type.name(),
                    link.id
                ))
            })?;
        remove_indexes.push(index);
    }
    remove_indexes.sort_unstable_by(|a, b| b.cmp(a));
    let mut ops: Vec<Value> = remove_indexes
        .into_iter()
        .map(|index| json!({"op": "remove", "path": format!("/relations/{index}")}))
        .collect();
    ops.extend(
        links
            .iter()
            .filter(|link| !existing.iter().any(|relation| matches_link(relation, link)))
            .map(|link| add_op(organization, link)),
    );
    Ok(ops)
}

/// The `relations` array for `wi show`/`create`/`update` output. Empty when the item carries no
/// relations.
pub(super) fn relations_output(
    item: &Value,
    organization: &Organization,
    project: &str,
) -> Result<Value> {
    let entries = item
        .get("relations")
        .and_then(Value::as_array)
        .map(|relations| {
            relations
                .iter()
                .map(|relation| relation_entry(relation, organization, project))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Value::Array(entries))
}

fn relation_entry(relation: &Value, organization: &Organization, project: &str) -> Result<Value> {
    let rel = relation
        .get("rel")
        .and_then(Value::as_str)
        .ok_or_else(invalid_work_item_response)?;
    let url = relation
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(invalid_work_item_response)?;
    let link_type = LinkType::from_reference_name(rel);
    let id = target_id(url);
    let output_url = match (link_type, id) {
        (Some(_), Some(id)) => work_item_url(organization, project, id)?,
        _ => url.to_owned(),
    };
    Ok(json!({
        "type": link_type.map(LinkType::name),
        "rel": rel,
        "id": id,
        "url": output_url,
        "name": relation
            .get("attributes")
            .and_then(|attributes| attributes.get("name"))
            .cloned()
            .unwrap_or(Value::Null),
    }))
}

fn add_op(organization: &Organization, link: &WorkItemLink) -> Value {
    json!({
        "op": "add",
        "path": "/relations/-",
        "value": {
            "rel": link.link_type.reference_name(),
            "url": work_item_api_url(organization, link.id),
        },
    })
}

/// Azure may return a relation's `rel` in a different case and its `url` with or without a
/// project GUID segment before the ID, so matching is case-insensitive and only looks at the
/// trailing path segment.
fn matches_link(relation: &Value, link: &WorkItemLink) -> bool {
    relation
        .get("rel")
        .and_then(Value::as_str)
        .is_some_and(|rel| rel.eq_ignore_ascii_case(link.link_type.reference_name()))
        && relation
            .get("url")
            .and_then(Value::as_str)
            .and_then(target_id)
            == Some(link.id)
}

/// The work item ID from a `.../_apis/wit/workItems/ID` relation URL; `None` for any other URL
/// (hyperlinks, attachments, artifact links), even one that happens to end in a number.
fn target_id(url: &str) -> Option<i64> {
    let mut segments = url.rsplit('/');
    let id = segments.next()?.parse::<i64>().ok().filter(|id| *id > 0)?;
    segments
        .next()?
        .eq_ignore_ascii_case("workItems")
        .then_some(id)
}

fn work_item_api_url(organization: &Organization, id: i64) -> String {
    format!("{}/_apis/wit/workItems/{id}", organization.url())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ado_core::ado_client::{HttpRequest, HttpResponse};
    use std::sync::Mutex;

    fn relation(rel: &str, url: &str) -> Value {
        json!({"rel": rel, "url": url})
    }

    fn link(link_type: LinkType, id: i64) -> WorkItemLink {
        WorkItemLink { link_type, id }
    }

    fn organization() -> Organization {
        Organization::parse("acme").unwrap()
    }

    #[test]
    fn update_ops_skips_adds_that_already_exist_case_and_guid_insensitively() {
        let item = json!({
            "relations": [
                relation(
                    "system.linktypes.dependency-reverse",
                    "https://dev.azure.com/acme/11111111-1111-1111-1111-111111111111/_apis/wit/workItems/12",
                ),
            ],
        });
        let ops = update_ops(
            7,
            &item,
            &organization(),
            &[link(LinkType::Predecessor, 12), link(LinkType::Related, 13)],
            &[],
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![json!({
                "op": "add",
                "path": "/relations/-",
                "value": {
                    "rel": "System.LinkTypes.Related",
                    "url": "https://dev.azure.com/acme/_apis/wit/workItems/13",
                },
            })]
        );
    }

    #[test]
    fn update_ops_emits_removes_in_descending_index_order_before_adds() {
        let item = json!({
            "relations": [
                relation(
                    "System.LinkTypes.Related",
                    "https://dev.azure.com/acme/_apis/wit/workItems/1",
                ),
                relation(
                    "System.LinkTypes.Dependency-Forward",
                    "https://dev.azure.com/acme/_apis/wit/workItems/2",
                ),
                relation(
                    "System.LinkTypes.Hierarchy-Reverse",
                    "https://dev.azure.com/acme/_apis/wit/workItems/3",
                ),
            ],
        });
        let ops = update_ops(
            7,
            &item,
            &organization(),
            &[link(LinkType::Child, 20)],
            &[link(LinkType::Related, 1), link(LinkType::Successor, 2)],
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![
                json!({"op": "remove", "path": "/relations/1"}),
                json!({"op": "remove", "path": "/relations/0"}),
                json!({
                    "op": "add",
                    "path": "/relations/-",
                    "value": {
                        "rel": "System.LinkTypes.Hierarchy-Forward",
                        "url": "https://dev.azure.com/acme/_apis/wit/workItems/20",
                    },
                }),
            ]
        );
    }

    #[test]
    fn update_ops_errors_naming_the_missing_remove_link_without_touching_adds() {
        let item = json!({"relations": []});
        let error = update_ops(
            7,
            &item,
            &organization(),
            &[link(LinkType::Related, 99)],
            &[link(LinkType::Predecessor, 12)],
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "Work item 7 has no predecessor link to 12.");
    }

    #[test]
    fn relations_output_is_empty_array_when_item_has_no_relations() {
        assert_eq!(
            relations_output(&json!({}), &organization(), "Project").unwrap(),
            json!([])
        );
    }

    #[test]
    fn relations_output_maps_known_link_types_to_friendly_names_and_work_item_urls() {
        let item = json!({
            "relations": [
                relation(
                    "System.LinkTypes.Dependency-Reverse",
                    "https://dev.azure.com/acme/_apis/wit/workItems/12",
                ),
            ],
        });
        let output = relations_output(&item, &organization(), "Project").unwrap();
        assert_eq!(
            output,
            json!([{
                "type": "predecessor",
                "rel": "System.LinkTypes.Dependency-Reverse",
                "id": 12,
                "url": "https://dev.azure.com/acme/Project/_workitems/edit/12",
                "name": null,
            }])
        );
    }

    #[test]
    fn relations_output_leaves_non_work_item_relations_with_the_raw_url_and_null_type() {
        let item = json!({
            "relations": [{
                "rel": "ArtifactLink",
                "url": "vstfs:///Git/PullRequestId/proj-guid%2Frepo-guid%2F42",
                "attributes": {"name": "Pull Request"},
            }],
        });
        let output = relations_output(&item, &organization(), "Project").unwrap();
        assert_eq!(
            output,
            json!([{
                "type": null,
                "rel": "ArtifactLink",
                "id": null,
                "url": "vstfs:///Git/PullRequestId/proj-guid%2Frepo-guid%2F42",
                "name": "Pull Request",
            }])
        );
    }

    #[test]
    fn target_id_only_reads_work_item_api_urls() {
        assert_eq!(
            target_id("https://dev.azure.com/acme/_apis/wit/workItems/12"),
            Some(12)
        );
        assert_eq!(
            target_id("https://dev.azure.com/acme/guid/_apis/wit/workitems/12"),
            Some(12)
        );
        assert_eq!(target_id("https://example.com/issues/12"), None);
    }

    /// Replays canned responses in order and records every request it sees, mirroring
    /// `pull_requests::management`'s test double for the same purpose.
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
        AdoClient::with_transport(organization(), "token", script)
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
    fn apply_create_appends_link_ops_after_the_field_patch() {
        let client = scripted(Script::new(vec![(200, json!({"id": 9, "fields": {}}))]));
        apply_create(
            &client,
            &organization(),
            "Project",
            "Task",
            vec![json!({"op": "add", "path": "/fields/System.Title", "value": "T"})],
            &[link(LinkType::Parent, 5)],
        )
        .unwrap();
        let seen = requests(&client);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "POST");
        assert_eq!(
            seen[0].1,
            "/acme/Project/_apis/wit/workitems/$Task?%24expand=relations&api-version=7.1"
        );
        assert_eq!(
            seen[0].2,
            Some(json!([
                {"op": "add", "path": "/fields/System.Title", "value": "T"},
                {
                    "op": "add",
                    "path": "/relations/-",
                    "value": {
                        "rel": "System.LinkTypes.Hierarchy-Reverse",
                        "url": "https://dev.azure.com/acme/_apis/wit/workItems/5",
                    },
                },
            ]))
        );
    }

    #[test]
    fn apply_update_fetches_relations_then_skips_existing_adds_and_patches_the_rest() {
        let client = scripted(Script::new(vec![
            (
                200,
                json!({"id": 7, "relations": [relation(
                    "System.LinkTypes.Related",
                    "https://dev.azure.com/acme/_apis/wit/workItems/1",
                )]}),
            ),
            (200, json!({"id": 7, "relations": []})),
        ]));
        apply_update(
            &client,
            &organization(),
            "Project",
            7,
            vec![json!({"op": "add", "path": "/fields/System.State", "value": "Active"})],
            &[link(LinkType::Related, 1), link(LinkType::Child, 2)],
            &[],
        )
        .unwrap();
        let seen = requests(&client);
        assert_eq!(seen.len(), 2);
        assert_eq!(
            (seen[0].0.as_str(), seen[0].1.as_str()),
            (
                "GET",
                "/acme/Project/_apis/wit/workitems/7?%24expand=relations&api-version=7.1"
            )
        );
        assert_eq!(seen[1].0, "PATCH");
        assert_eq!(
            seen[1].2,
            Some(json!([
                {"op": "add", "path": "/fields/System.State", "value": "Active"},
                {
                    "op": "add",
                    "path": "/relations/-",
                    "value": {
                        "rel": "System.LinkTypes.Hierarchy-Forward",
                        "url": "https://dev.azure.com/acme/_apis/wit/workItems/2",
                    },
                },
            ]))
        );
    }

    #[test]
    fn apply_update_is_a_single_read_when_every_link_already_exists_and_nothing_else_changes() {
        let client = scripted(Script::new(vec![(
            200,
            json!({"id": 7, "relations": [relation(
                "System.LinkTypes.Related",
                "https://dev.azure.com/acme/_apis/wit/workItems/1",
            )]}),
        )]));
        let item = apply_update(
            &client,
            &organization(),
            "Project",
            7,
            vec![],
            &[link(LinkType::Related, 1)],
            &[],
        )
        .unwrap();
        assert_eq!(item["id"], 7);
        let seen = requests(&client);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "GET");
    }

    #[test]
    fn apply_update_rejects_a_missing_remove_link_without_sending_a_patch() {
        let client = scripted(Script::new(vec![(200, json!({"id": 7, "relations": []}))]));
        let error = apply_update(
            &client,
            &organization(),
            "Project",
            7,
            vec![],
            &[],
            &[link(LinkType::Predecessor, 12)],
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "Work item 7 has no predecessor link to 12.");
        let seen = requests(&client);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "GET");
    }
}
