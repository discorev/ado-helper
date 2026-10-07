mod common;

use common::*;

#[test]
fn identity_uses_account_property_fallback() {
    let m = Mock::new(|r, _| {
        assert_eq!(r.url.path(), "/acme/_apis/connectionData");
        assert_eq!(query(r)["connectOptions"], "1");
        assert_eq!(r.headers["Authorization"], "Basic OnRlc3QtdG9rZW4=");
        Ok(response(
            r,
            200,
            &json!({"authenticatedUser":{"id":"identity-id","providerDisplayName":"Ada Lovelace","properties":{"Account":{"$value":"ada@example.test"}}}}),
            &[],
        ))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "test-token", m);
    assert_eq!(
        c.identity().unwrap(),
        AdoIdentity {
            id: "identity-id".into(),
            display_name: "Ada Lovelace".into(),
            unique_name: "ada@example.test".into(),
        }
    );
    assert_eq!(c.transport.count(), 1)
}
#[test]
fn pull_request_uses_org_level_endpoint() {
    let m = Mock::new(|r, _| {
        assert_eq!(r.url.path(), "/acme/_apis/git/pullrequests/99");
        assert_eq!(query(r)["api-version"], "7.1");
        Ok(response(r, 200, &json!({"pullRequestId":99}), &[]))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    assert_eq!(c.pull_request(99).unwrap()["pullRequestId"], 99)
}
#[test]
fn file_content_pins_commit_and_escapes_path() {
    let commit = "ABCDEF0123456789ABCDEF0123456789ABCDEF01";
    let m = Mock::new(move |r, _| {
        assert_eq!(r.method, "GET");
        assert_eq!(r.headers["Accept"], "application/json");
        assert!(
            r.url
                .as_str()
                .contains("/My%20Project/_apis/git/repositories/repo%2Fname/items")
        );
        let q = query(r);
        assert_eq!(q["path"], "/Sources/A & B.swift");
        assert_eq!(q["includeContent"], "true");
        assert_eq!(q["includeContentMetadata"], "true");
        assert_eq!(q["versionDescriptor.versionType"], "commit");
        assert_eq!(q["versionDescriptor.version"], commit.to_ascii_lowercase());
        assert_eq!(q["api-version"], "7.1");
        Ok(response(
            r,
            200,
            &json!({"gitObjectType":"blob","isFolder":false,"contentMetadata":{"isBinary":false},"content":"first\nsecond\n"}),
            &[],
        ))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    assert_eq!(
        c.file_content("My Project", "repo/name", "Sources/A & B.swift", commit)
            .unwrap(),
        "first\nsecond\n"
    )
}
#[test]
fn file_content_rejects_binary_and_folder() {
    for p in [
        json!({"gitObjectType":"blob","isFolder":false,"contentMetadata":{"isBinary":true},"content":"AAEC"}),
        json!({"gitObjectType":"tree","isFolder":true,"contentMetadata":{"isBinary":false}}),
    ] {
        let m = Mock::new(move |r, _| Ok(response(r, 200, &p, &[])));
        let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
        assert_eq!(
            c.file_content("Project", "Repo", "/file.bin", &"a".repeat(40))
                .unwrap_err()
                .to_string(),
            "Azure DevOps returned a response in an unexpected format."
        )
    }
}
#[test]
fn file_content_rejects_invalid_commit_without_request() {
    let m = Mock::new(|_, _| panic!("request"));
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    assert_eq!(
        c.file_content("Project", "Repo", "/file.swift", "main")
            .unwrap_err()
            .to_string(),
        "Commit must be a 40-character hexadecimal SHA."
    );
    assert_eq!(c.transport.count(), 0)
}
#[test]
fn path_components_and_source_branch_escaped_as_data() {
    let m = Mock::new(|r, _| {
        assert!(
            r.url
                .as_str()
                .contains("/A%20Project/_apis/git/repositories/repo%2Fname/pullrequests")
        );
        assert_eq!(
            query(r)["searchCriteria.sourceRefName"],
            "refs/heads/feature & fix"
        );
        assert_eq!(query(r)["searchCriteria.status"], "active");
        Ok(response(r, 200, &json!({"value":[]}), &[]))
    });
    AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m)
        .list_pull_requests("A Project", "repo/name", Some("refs/heads/feature & fix"))
        .unwrap();
}
#[test]
fn changes_flattens_next_skip_pages() {
    let m = Mock::new(|r, i| {
        if i == 0 {
            let q = query(r);
            assert_eq!(q["$top"], "2000");
            assert!(!q.contains_key("$skip"));
            Ok(response(
                r,
                200,
                &json!({"changeEntries":[{"changeTrackingId":1}],"nextSkip":1,"nextTop":25}),
                &[],
            ))
        } else {
            let q = query(r);
            assert_eq!(q["$top"], "25");
            assert_eq!(q["$skip"], "1");
            Ok(response(
                r,
                200,
                &json!({"changeEntries":[{"changeTrackingId":2}],"nextSkip":0}),
                &[],
            ))
        }
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    let v = c.changes("Project", "Repo", 4, 3).unwrap();
    assert_eq!(
        v.iter()
            .map(|item| item["changeTrackingId"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(c.transport.count(), 2)
}
#[test]
fn threads_follows_continuation_header() {
    let token = "next token&opaque=yes";
    let m = Mock::new(move |r, i| {
        if i == 0 {
            assert!(!query(r).contains_key("continuationToken"));
            Ok(response(
                r,
                200,
                &json!({"value":[{"id":1}]}),
                &[("x-ms-continuationtoken", token)],
            ))
        } else {
            assert_eq!(query(r)["continuationToken"], token);
            Ok(response(r, 200, &json!({"value":[{"id":2}]}), &[]))
        }
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    let threads = c.threads("Project", "Repo", 4).unwrap();
    assert_eq!(
        threads
            .iter()
            .map(|item| item["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(c.transport.count(), 2)
}
#[test]
fn create_thread_builds_side_and_iteration_context() {
    let m = Mock::new(|r, _| {
        assert_eq!(r.method, "POST");
        let b: Value = serde_json::from_slice(r.body.as_ref().unwrap()).unwrap();
        assert_eq!(b["comments"][0]["content"], "Please adjust this");
        assert_eq!(b["threadContext"]["filePath"], "/Sources/App.swift");
        assert!(b["threadContext"]["leftFileStart"].is_null());
        assert_eq!(b["threadContext"]["rightFileStart"]["line"], 12);
        assert_eq!(b["threadContext"]["rightFileEnd"]["line"], 14);
        assert_eq!(b["pullRequestThreadContext"]["changeTrackingId"], 31);
        assert_eq!(
            b["pullRequestThreadContext"]["iterationContext"]["firstComparingIteration"],
            5
        );
        assert_eq!(
            b["pullRequestThreadContext"]["iterationContext"]["secondComparingIteration"],
            5
        );
        Ok(response(r, 200, &json!({"id":123}), &[]))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "token", m);
    assert_eq!(
        c.create_thread(
            "Project",
            "Repo",
            4,
            "Please adjust this",
            "/Sources/App.swift",
            12,
            14,
            "right",
            5,
            31
        )
        .unwrap()["id"],
        123
    );
    assert_eq!(c.transport.count(), 1)
}
#[test]
fn mutation_not_retried_and_error_hides_body_token() {
    let secret = "super-secret-token";
    let m = Mock::new(move |r, _| {
        Ok(HttpResponse {
            status: 503,
            url: r.url.clone(),
            headers: HashMap::new(),
            body: format!("private {secret}").into_bytes(),
        })
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), secret, m);
    let e = c
        .create_thread("Project", "Repo", 4, "body", "/file", 1, 1, "left", 1, 1)
        .unwrap_err()
        .to_string();
    assert!(e.contains("503"));
    assert!(!e.contains(secret));
    assert!(!e.contains("private"));
    assert_eq!(c.transport.count(), 1)
}
#[test]
fn cross_host_response_rejected_without_token() {
    let m = Mock::new(|_r, _| {
        Ok(HttpResponse {
            status: 200,
            url: url::Url::parse("https://evil.example/capture").unwrap(),
            headers: HashMap::new(),
            body: json!({"value":[]}).to_string().into_bytes(),
        })
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "redirect-secret", m);
    let e = c.threads("Project", "Repo", 4).unwrap_err().to_string();
    assert_eq!(
        e,
        "Azure DevOps redirected the request. Redirects are disabled for authenticated requests."
    );
    assert!(!e.contains("redirect-secret"))
}
#[test]
fn same_host_redirect_also_rejected() {
    let m = Mock::new(|r, _| {
        let mut u = r.url.clone();
        u.set_path("/another-organization/_apis/git/pullrequests/4");
        Ok(HttpResponse {
            status: 200,
            url: u,
            headers: HashMap::new(),
            body: json!({"pullRequestId":4}).to_string().into_bytes(),
        })
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "secret", m);
    assert_eq!(
        c.pull_request(4).unwrap_err().to_string(),
        "Azure DevOps redirected the request. Redirects are disabled for authenticated requests."
    )
}
#[test]
fn list_pull_requests_stops_repeating_full_page() {
    let page = Value::Array((0..100).map(|i| json!({"pullRequestId":i})).collect());
    let m = Mock::new(move |r, i| {
        assert_eq!(query(r)["$skip"], (i * 100).to_string());
        Ok(response(r, 200, &json!({"value":page}), &[]))
    });
    let c = AdoClient::with_transport(Organization::parse("acme").unwrap(), "secret", m);
    assert_eq!(
        c.list_pull_requests("Project", "Repo", None)
            .unwrap_err()
            .to_string(),
        "Azure DevOps returned an invalid or repeating pagination cursor."
    );
    assert_eq!(c.transport.count(), 2)
}
