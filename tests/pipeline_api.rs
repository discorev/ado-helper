mod common;
use ado_core::ado_client::builds::BuildQuery;
use common::*;

fn client(mock: Mock) -> AdoClient<Mock> {
    AdoClient::with_transport(Organization::parse("acme").unwrap(), "fake-token", mock)
}

#[test]
fn definitions_include_full_properties_filter_and_continuation() {
    let c = client(Mock::new(|r, index| {
        assert_eq!(r.method, "GET");
        assert_eq!(
            r.url.path(),
            "/acme/Example%20Project/_apis/build/definitions"
        );
        let q = query(r);
        assert_eq!(q["api-version"], "7.1");
        assert_eq!(q["includeAllProperties"], "true");
        assert_eq!(q["name"], "example-*");
        if index == 0 {
            assert!(!q.contains_key("continuationToken"));
            Ok(response(
                r,
                200,
                &json!({"value":[{"id":1,"name":"example-ci"}]}),
                &[("x-ms-continuationtoken", "next page")],
            ))
        } else {
            assert_eq!(q["continuationToken"], "next page");
            Ok(response(
                r,
                200,
                &json!({"value":[{"id":2,"name":"example-release"}]}),
                &[],
            ))
        }
    }));
    let definitions = c
        .build_definitions("Example Project", Some("example-*"))
        .unwrap();
    assert_eq!(definitions.len(), 2);
    assert_eq!(c.transport.count(), 2);
}

#[test]
fn definitions_without_filter_and_invalid_or_repeated_pagination() {
    let c = client(Mock::new(|r, _| {
        assert!(!query(r).contains_key("name"));
        Ok(response(r, 200, &json!({"value":[]}), &[]))
    }));
    assert!(
        c.build_definitions("Example Project", None)
            .unwrap()
            .is_empty()
    );
    let c = client(Mock::new(|r, _| {
        Ok(response(
            r,
            200,
            &json!({"value":[]}),
            &[("x-ms-continuationtoken", "same")],
        ))
    }));
    assert!(c.build_definitions("Example Project", None).is_err());
    assert_eq!(c.transport.count(), 2);
    let c = client(Mock::new(|r, _| Ok(response(r, 200, &json!({}), &[]))));
    assert!(c.build_definitions("Example Project", None).is_err());
}

#[test]
fn builds_send_all_filters_in_newest_first_order_and_limit_across_pages() {
    let c = client(Mock::new(|r, index| {
        assert_eq!(r.url.path(), "/acme/Example%20Project/_apis/build/builds");
        let q = query(r);
        assert_eq!(q["api-version"], "7.1");
        assert_eq!(q["queryOrder"], "queueTimeDescending");
        assert_eq!(q["definitions"], "4");
        assert_eq!(q["branchName"], "refs/heads/main");
        assert_eq!(q["statusFilter"], "completed");
        assert_eq!(q["resultFilter"], "failed");
        if index == 0 {
            assert_eq!(q["$top"], "3");
            Ok(response(
                r,
                200,
                &json!({"value":[{"id":3},{"id":2}]}),
                &[("x-ms-continuationtoken", "more")],
            ))
        } else {
            assert_eq!(q["$top"], "1");
            assert_eq!(q["continuationToken"], "more");
            Ok(response(
                r,
                200,
                &json!({"value":[{"id":1},{"id":99}]}),
                &[("x-ms-continuationtoken", "unused")],
            ))
        }
    }));
    let builds = c
        .builds(
            "Example Project",
            &BuildQuery {
                definition: Some(4),
                branch: Some("refs/heads/main"),
                status: Some("completed"),
                result: Some("failed"),
                top: 3,
            },
        )
        .unwrap();
    assert_eq!(
        builds,
        vec![json!({"id":3}), json!({"id":2}), json!({"id":1})]
    );
    assert_eq!(c.transport.count(), 2);
}

#[test]
fn builds_omit_unspecified_filters() {
    let c = client(Mock::new(|r, _| {
        assert_eq!(query(r).len(), 3);
        assert_eq!(query(r)["$top"], "25");
        Ok(response(r, 200, &json!({"value":[]}), &[]))
    }));
    assert!(
        c.builds(
            "Example Project",
            &BuildQuery {
                definition: None,
                branch: None,
                status: None,
                result: None,
                top: 25
            }
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn build_and_timeline_use_correct_paths_and_api_version() {
    let c = client(Mock::new(|r, index| {
        assert_eq!(r.method, "GET");
        assert_eq!(query(r)["api-version"], "7.1");
        match index {
            0 => {
                assert_eq!(
                    r.url.path(),
                    "/acme/Example%20Project/_apis/build/builds/123"
                );
                Ok(response(r, 200, &json!({"id":123}), &[]))
            }
            1 => {
                assert_eq!(
                    r.url.path(),
                    "/acme/Example%20Project/_apis/build/builds/123/timeline"
                );
                Ok(response(
                    r,
                    200,
                    &json!({"records":[{"id":"task","type":"Task"}]}),
                    &[],
                ))
            }
            _ => Ok(response(r, 200, &json!({}), &[])),
        }
    }));
    assert_eq!(c.build("Example Project", 123).unwrap()["id"], 123);
    assert_eq!(
        c.build_timeline("Example Project", 123).unwrap()[0]["type"],
        "Task"
    );
    assert!(c.build_timeline("Example Project", 123).is_err());
}

#[test]
fn logs_request_plain_text_and_preserve_exact_bytes() {
    let c = client(Mock::new(|r, _| {
        assert_eq!(r.method, "GET");
        assert_eq!(
            r.url.path(),
            "/acme/Example%20Project/_apis/build/builds/123/logs/7"
        );
        assert_eq!(query(r)["api-version"], "7.1");
        assert_eq!(r.headers["Accept"], "text/plain");
        Ok(HttpResponse {
            status: 200,
            url: r.url.clone(),
            headers: HashMap::new(),
            body: "héllo\r\nexample\n".as_bytes().to_vec(),
        })
    }));
    assert_eq!(
        c.build_log("Example Project", 123, 7).unwrap(),
        "héllo\r\nexample\n"
    );
    let c = client(Mock::new(|r, _| {
        Ok(HttpResponse {
            status: 200,
            url: r.url.clone(),
            headers: HashMap::new(),
            body: vec![255],
        })
    }));
    assert!(
        c.build_log("Example Project", 123, 7)
            .unwrap_err()
            .0
            .contains("UTF-8")
    );
}

#[test]
fn test_runs_filter_build_uri_and_page_with_offsets() {
    let c = client(Mock::new(|r, index| {
        assert_eq!(r.url.path(), "/acme/Example%20Project/_apis/test/runs");
        let q = query(r);
        assert_eq!(q["api-version"], "7.1");
        assert_eq!(q["buildUri"], "vstfs:///Build/Build/123");
        assert_eq!(q["$top"], "100");
        assert_eq!(q["$skip"], (index * 100).to_string());
        let rows = if index == 0 {
            (1..=100).map(|id| json!({"id":id})).collect::<Vec<_>>()
        } else {
            vec![json!({"id":101})]
        };
        Ok(response(r, 200, &json!({"value":rows}), &[]))
    }));
    assert_eq!(
        c.build_test_runs("Example Project", 123).unwrap().len(),
        101
    );
    assert_eq!(c.transport.count(), 2);
}

#[test]
fn test_statistics_use_the_run_id_and_api_version() {
    let c = client(Mock::new(|r, _| {
        assert_eq!(
            r.url.path(),
            "/acme/Example%20Project/_apis/test/runs/11/statistics"
        );
        assert_eq!(query(r)["api-version"], "7.1");
        Ok(response(
            r,
            200,
            &json!({"runStatistics":[{"outcome":"Failed", "count":2}]}),
            &[],
        ))
    }));
    assert_eq!(
        c.test_run_statistics("Example Project", 11).unwrap()["runStatistics"][0]["count"],
        2
    );
}

#[test]
fn failed_test_results_filter_outcomes_and_page_before_client_filtering() {
    let c = client(Mock::new(|r, index| {
        assert_eq!(
            r.url.path(),
            "/acme/Example%20Project/_apis/test/Runs/11/results"
        );
        let q = query(r);
        assert_eq!(q["api-version"], "7.1");
        assert_eq!(q["outcomes"], "Failed");
        assert_eq!(q["$top"], "100");
        assert_eq!(q["$skip"], (index * 100).to_string());
        let rows = if index == 0 {
            (0..100)
                .map(|id| json!({"id":id,"outcome": if id == 0 { "Passed" } else { "Failed" }}))
                .collect::<Vec<_>>()
        } else {
            vec![json!({"id":100,"outcome":"Failed"})]
        };
        Ok(response(r, 200, &json!({"value":rows}), &[]))
    }));
    let results = c.test_results("Example Project", 11, false).unwrap();
    assert_eq!(results.len(), 100);
    assert_eq!(results[0]["id"], 1);
    assert_eq!(results[99]["id"], 100);
    assert!(results.iter().all(|result| result["outcome"] == "Failed"));
    assert_eq!(c.transport.count(), 2);
}

#[test]
fn all_test_results_omit_outcome_filter_and_keep_all_outcomes() {
    let c = client(Mock::new(|r, _| {
        assert!(!query(r).contains_key("outcomes"));
        Ok(response(
            r,
            200,
            &json!({"value":[{"outcome":"Failed"}, {"outcome":"Passed"}, {"outcome":"NotExecuted"}]}),
            &[],
        ))
    }));
    assert_eq!(
        c.test_results("Example Project", 11, true).unwrap().len(),
        3
    );
}

#[test]
fn test_pagination_handles_exact_page_and_detects_repeated_pages() {
    let c = client(Mock::new(|r, index| {
        let rows = if index == 0 {
            (0..100).map(|id| json!({"id":id})).collect::<Vec<_>>()
        } else {
            vec![]
        };
        Ok(response(r, 200, &json!({"value":rows}), &[]))
    }));
    assert_eq!(
        c.test_results("Example Project", 11, true).unwrap().len(),
        100
    );
    assert_eq!(c.transport.count(), 2);
    let c = client(Mock::new(|r, _| {
        Ok(response(
            r,
            200,
            &json!({"value":(0..100).map(|id| json!({"id":id})).collect::<Vec<_>>()}),
            &[],
        ))
    }));
    assert!(c.test_results("Example Project", 11, true).is_err());
    assert_eq!(c.transport.count(), 2);
}

#[test]
fn build_and_test_auth_errors_include_scope_renewal_and_server_message() {
    for status in [401, 403] {
        let c = client(Mock::new(move |r, _| {
            Ok(response(
                r,
                status,
                &json!({"message":"Example access denial"}),
                &[],
            ))
        }));
        for error in [
            c.build("Example Project", 123).unwrap_err(),
            c.build_log("Example Project", 123, 7).unwrap_err(),
        ] {
            assert!(error.0.contains("Build — Read (vso.build)"));
            assert!(error.0.contains("ado auth update NAME"));
            assert!(error.0.contains("Example access denial"));
        }
        for error in [
            c.build_test_runs("Example Project", 123).unwrap_err(),
            c.test_run_statistics("Example Project", 11).unwrap_err(),
            c.test_results("Example Project", 11, false).unwrap_err(),
        ] {
            assert!(error.0.contains("Test Management — Read (vso.test)"));
            assert!(error.0.contains("ado auth update NAME"));
            assert!(error.0.contains("Example access denial"));
        }
    }
}

#[test]
fn pipeline_client_validation_prevents_requests() {
    let c = client(Mock::new(|_, _| {
        panic!("Invalid input should not send HTTP requests")
    }));
    assert!(c.build_definitions("", None).is_err());
    assert!(c.build("Example Project", 0).is_err());
    assert!(c.build_timeline("Example Project", -1).is_err());
    assert!(c.build_log("Example Project", 123, 0).is_err());
    assert!(c.build_test_runs("Example Project", 0).is_err());
    assert!(c.test_results("Example Project", -1, false).is_err());
    assert!(c.test_run_statistics("Example Project", 0).is_err());
    assert!(
        c.builds(
            "Example Project",
            &BuildQuery {
                definition: None,
                branch: None,
                status: None,
                result: None,
                top: 0
            }
        )
        .is_err()
    );
    assert_eq!(c.transport.count(), 0);
}

#[test]
fn pipeline_scope_instructions_include_both_read_scopes() {
    let text = auth::scope_instructions(&Organization::parse("acme").unwrap());
    assert!(text.contains("Required for pipelines: Build — Read (vso.build)"));
    assert!(text.contains("Required for pipeline tests: Test Management — Read (vso.test)"));
    assert!(!text.contains("Not needed by current ado commands"));
}
