use crate::{
    Error, Result, git::is_commit_sha, models::AdoIdentity, organization::Organization,
    platform::is_swift_control,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Method, blocking::Client};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use url::Url;

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: Url,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub url: Url,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}
pub trait HttpTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse>;
}
pub struct ReqwestTransport {
    client: Client,
}
impl ReqwestTransport {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| Error("The Azure DevOps request could not be completed.".into()))?;
        Ok(Self { client })
    }
}
impl HttpTransport for ReqwestTransport {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse> {
        let method = Method::from_bytes(r.method.as_bytes()).map_err(|_| invalid_request())?;
        let mut req = self.client.request(method, r.url.clone());
        for (k, v) in &r.headers {
            req = req.header(k, v)
        }
        if let Some(b) = &r.body {
            req = req.body(b.clone())
        }
        let res = req
            .send()
            .map_err(|_| Error("The Azure DevOps request could not be completed.".into()))?;
        let status = res.status().as_u16();
        let url = res.url().clone();
        let headers = res
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|v| (k.as_str().to_ascii_lowercase(), v.to_owned()))
            })
            .collect();
        let body = res
            .bytes()
            .map_err(|_| Error("The Azure DevOps request could not be completed.".into()))?
            .to_vec();
        Ok(HttpResponse {
            status,
            url,
            headers,
            body,
        })
    }
}

pub struct AdoClient<T: HttpTransport = ReqwestTransport> {
    organization: Organization,
    authorization: String,
    pub transport: T,
}
impl AdoClient<ReqwestTransport> {
    pub fn new(organization: Organization, token: &str) -> Result<Self> {
        Ok(Self::with_transport(
            organization,
            token,
            ReqwestTransport::new()?,
        ))
    }
}
impl<T: HttpTransport> AdoClient<T> {
    pub fn with_transport(organization: Organization, token: &str, transport: T) -> Self {
        Self {
            organization,
            authorization: format!("Basic {}", STANDARD.encode(format!(":{token}"))),
            transport,
        }
    }
    pub fn identity(&self) -> Result<AdoIdentity> {
        let object = self.get_object(
            &["_apis", "connectionData"],
            &[
                ("connectOptions", "1"),
                ("lastChangeId", "-1"),
                ("lastChangeId64", "-1"),
            ],
            false,
        )?;
        let user = object
            .get("authenticatedUser")
            .and_then(Value::as_object)
            .ok_or_else(invalid_response)?;
        let id = nonempty(user.get("id")).ok_or_else(invalid_response)?;
        let display_name = nonempty(user.get("providerDisplayName"))
            .or_else(|| nonempty(user.get("displayName")))
            .ok_or_else(invalid_response)?;
        let unique_name = nonempty(user.get("uniqueName"))
            .or_else(|| {
                user.get("properties")
                    .and_then(|p| p.get("Account"))
                    .and_then(|a| nonempty(a.get("$value")).or_else(|| nonempty(a.get("value"))))
            })
            .ok_or_else(invalid_response)?;
        Ok(AdoIdentity {
            id,
            display_name,
            unique_name,
        })
    }
    pub fn pull_request(&self, id: i64) -> Result<Value> {
        if id <= 0 {
            return Err(Error("Pull request ID must be greater than zero.".into()));
        }
        self.get_object(
            &["_apis", "git", "pullrequests", &id.to_string()],
            &[],
            true,
        )
    }
    pub fn file_content(
        &self,
        project: &str,
        repo: &str,
        path: &str,
        commit: &str,
    ) -> Result<String> {
        self.validate_resource(project, repo, 1)?;
        if !is_commit_sha(commit) {
            return Err(Error(
                "Commit must be a 40-character hexadecimal SHA.".into(),
            ));
        }
        let normalized = if path.starts_with('/') {
            path.into()
        } else {
            format!("/{path}")
        };
        if normalized.len() <= 1
            || normalized.chars().any(is_swift_control)
            || normalized.contains("//")
            || normalized.split('/').any(|p| p == "." || p == "..")
        {
            return Err(Error(
                "File path must be a valid repository-relative path.".into(),
            ));
        }
        let lower = commit.to_ascii_lowercase();
        let obj = self.get_object(
            &[project, "_apis", "git", "repositories", repo, "items"],
            &[
                ("path", &normalized),
                ("includeContent", "true"),
                ("includeContentMetadata", "true"),
                ("versionDescriptor.versionType", "commit"),
                ("versionDescriptor.version", &lower),
            ],
            true,
        )?;
        if obj.get("isFolder").and_then(Value::as_bool) == Some(true)
            || obj
                .get("gitObjectType")
                .and_then(Value::as_str)
                .is_some_and(|x| x.eq_ignore_ascii_case("tree"))
            || obj
                .pointer("/contentMetadata/isBinary")
                .and_then(Value::as_bool)
                == Some(true)
            || obj.get("isBinary").and_then(Value::as_bool) == Some(true)
        {
            return Err(invalid_response());
        }
        obj.get("content")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(invalid_response)
    }
    pub fn threads(&self, project: &str, repo: &str, id: i64) -> Result<Vec<Value>> {
        self.validate_resource(project, repo, id)?;
        self.continuation(&[
            project,
            "_apis",
            "git",
            "repositories",
            repo,
            "pullRequests",
            &id.to_string(),
            "threads",
        ])
    }
    pub fn iterations(&self, project: &str, repo: &str, id: i64) -> Result<Vec<Value>> {
        self.validate_resource(project, repo, id)?;
        self.continuation(&[
            project,
            "_apis",
            "git",
            "repositories",
            repo,
            "pullRequests",
            &id.to_string(),
            "iterations",
        ])
    }
    pub fn changes(
        &self,
        project: &str,
        repo: &str,
        id: i64,
        iteration: i64,
    ) -> Result<Vec<Value>> {
        self.validate_resource(project, repo, id)?;
        if iteration <= 0 {
            return Err(Error("Iteration must be greater than zero.".into()));
        }
        let mut results = Vec::new();
        let mut skip: Option<i64> = None;
        let mut top: i64 = 2000;
        let mut seen = HashSet::new();
        loop {
            let mut owned = vec![("$top".to_owned(), top.to_string())];
            if let Some(s) = skip {
                if s <= 0 || !seen.insert(s) {
                    return Err(pagination());
                }
                owned.push(("$skip".into(), s.to_string()));
            }
            let refs: Vec<(&str, &str)> = owned
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect();
            let obj = self.get_object(
                &[
                    project,
                    "_apis",
                    "git",
                    "repositories",
                    repo,
                    "pullRequests",
                    &id.to_string(),
                    "iterations",
                    &iteration.to_string(),
                    "changes",
                ],
                &refs,
                true,
            )?;
            let page = obj
                .get("changeEntries")
                .and_then(Value::as_array)
                .ok_or_else(invalid_response)?;
            results.extend(page.iter().cloned());
            let next = obj.get("nextSkip").and_then(integer).unwrap_or(0);
            if next == 0 {
                break;
            }
            let nt = obj.get("nextTop").and_then(integer).unwrap_or(2000);
            if next <= 0 || nt <= 0 || nt > 2000 {
                return Err(pagination());
            }
            skip = Some(next);
            top = nt;
        }
        Ok(results)
    }
    pub fn list_pull_requests(
        &self,
        project: &str,
        repo: &str,
        branch: Option<&str>,
    ) -> Result<Vec<Value>> {
        self.validate_resource(project, repo, 1)?;
        if branch == Some("") {
            return Err(Error("Source branch must not be empty.".into()));
        }
        let mut results = Vec::new();
        let mut seen = HashSet::new();
        let mut skip: i64 = 0;
        loop {
            let skip_string = skip.to_string();
            let mut q = vec![
                ("$top", "100"),
                ("$skip", skip_string.as_str()),
                ("searchCriteria.status", "active"),
            ];
            if let Some(b) = branch {
                q.push(("searchCriteria.sourceRefName", b));
            }
            let obj = self.get_object(
                &[
                    project,
                    "_apis",
                    "git",
                    "repositories",
                    repo,
                    "pullrequests",
                ],
                &q,
                true,
            )?;
            let page = obj
                .get("value")
                .and_then(Value::as_array)
                .ok_or_else(invalid_response)?;
            results.extend(page.iter().cloned());
            if page.len() != 100 {
                break;
            }
            let fingerprint = serde_json::to_vec(page).map_err(|_| invalid_response())?;
            if !seen.insert(fingerprint) {
                return Err(pagination());
            }
            skip = skip.checked_add(100).ok_or_else(pagination)?;
        }
        Ok(results)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn create_thread(
        &self,
        project: &str,
        repo: &str,
        id: i64,
        body: &str,
        path: &str,
        start: i64,
        end: i64,
        side: &str,
        iteration: i64,
        change_id: i64,
    ) -> Result<Value> {
        self.validate_resource(project, repo, id)?;
        if body.is_empty() {
            return Err(Error("Comment body must not be empty.".into()));
        }
        if path.is_empty() {
            return Err(Error("Comment path must not be empty.".into()));
        }
        if start <= 0 || end < start {
            return Err(Error(
                "Comment line range must use positive lines with the end at or after the start."
                    .into(),
            ));
        }
        if side != "left" && side != "right" {
            return Err(Error("Comment side must be left or right.".into()));
        }
        if iteration <= 0 || change_id <= 0 {
            return Err(Error(
                "Iteration and change tracking ID must be greater than zero.".into(),
            ));
        }
        let mut context = json!({"filePath":path,"leftFileStart":null,"leftFileEnd":null,"rightFileStart":null,"rightFileEnd":null});
        context[format!("{side}FileStart")] = json!({"line":start,"offset":1});
        context[format!("{side}FileEnd")] = json!({"line":end,"offset":1});
        let payload = json!({"comments":[{"parentCommentId":0,"content":body,"commentType":1}],"status":1,"threadContext":context,"pullRequestThreadContext":{"changeTrackingId":change_id,"iterationContext":{"firstComparingIteration":iteration,"secondComparingIteration":iteration}}});
        self.send_object(
            "POST",
            &[
                project,
                "_apis",
                "git",
                "repositories",
                repo,
                "pullRequests",
                &id.to_string(),
                "threads",
            ],
            &payload,
        )
    }
    fn continuation(&self, path: &[&str]) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        let mut seen = HashSet::new();
        loop {
            let q: Vec<(&str, &str)> = token
                .as_deref()
                .map(|v| vec![("continuationToken", v)])
                .unwrap_or_default();
            if let Some(t) = &token
                && (t.is_empty() || !seen.insert(t.clone()))
            {
                return Err(pagination());
            }
            let (obj, res) = self.get_object_response(path, &q, true)?;
            out.extend(
                obj.get("value")
                    .and_then(Value::as_array)
                    .ok_or_else(invalid_response)?
                    .iter()
                    .cloned(),
            );
            token = res.headers.get("x-ms-continuationtoken").cloned();
            if token.is_none() {
                break;
            }
        }
        Ok(out)
    }
    fn validate_resource(&self, p: &str, r: &str, id: i64) -> Result<()> {
        if p.is_empty() || r.is_empty() {
            Err(Error("Project and repository must not be empty.".into()))
        } else if id <= 0 {
            Err(Error("Pull request ID must be greater than zero.".into()))
        } else {
            Ok(())
        }
    }
    fn get_object(&self, path: &[&str], query: &[(&str, &str)], api: bool) -> Result<Value> {
        Ok(self.get_object_response(path, query, api)?.0)
    }
    fn get_object_response(
        &self,
        path: &[&str],
        query: &[(&str, &str)],
        api: bool,
    ) -> Result<(Value, HttpResponse)> {
        let req = self.make_request("GET", path, query, api)?;
        let res = self.perform(&req)?;
        let obj = serde_json::from_slice::<Value>(&res.body).map_err(|_| invalid_response())?;
        if !obj.is_object() {
            return Err(invalid_response());
        }
        Ok((obj, res))
    }
    fn send_object(&self, method: &str, path: &[&str], body: &Value) -> Result<Value> {
        let mut req = self.make_request(method, path, &[], true)?;
        req.headers
            .insert("Content-Type".into(), "application/json".into());
        req.body = Some(serde_json::to_vec(body).map_err(|_| invalid_request())?);
        let res = self.perform(&req)?;
        let obj = serde_json::from_slice::<Value>(&res.body).map_err(|_| invalid_response())?;
        if obj.is_object() {
            Ok(obj)
        } else {
            Err(invalid_response())
        }
    }
    pub fn make_request(
        &self,
        method: &str,
        path: &[&str],
        query: &[(&str, &str)],
        api: bool,
    ) -> Result<HttpRequest> {
        let mut url = Url::parse("https://dev.azure.com").map_err(|_| invalid_request())?;
        {
            let mut ps = url.path_segments_mut().map_err(|_| invalid_request())?;
            ps.push(&self.organization.name);
            for p in path {
                ps.push(p);
            }
        }
        {
            let mut qp = url.query_pairs_mut();
            for (k, v) in query {
                qp.append_pair(k, v);
            }
            if api {
                qp.append_pair("api-version", "7.1");
            }
        }
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), self.authorization.clone());
        headers.insert("Accept".into(), "application/json".into());
        Ok(HttpRequest {
            method: method.into(),
            url,
            headers,
            body: None,
        })
    }
    fn perform(&self, req: &HttpRequest) -> Result<HttpResponse> {
        let res = self.transport.send(req)?;
        if res.url.as_str() != req.url.as_str() {
            return Err(Error("Azure DevOps redirected the request. Redirects are disabled for authenticated requests.".into()));
        }
        if !(200..300).contains(&res.status) {
            let path = percent_encoding::percent_decode_str(req.url.path()).decode_utf8_lossy();
            return Err(Error(match res.status{401=>"Azure DevOps rejected authentication (HTTP 401). Renew the affected profile with 'ado auth update NAME'.".into(),403=>"Azure DevOps denied access (HTTP 403). Check the profile's PAT scopes and account permissions for this repository.".into(),s=>format!("Azure DevOps returned HTTP {s} for {} {path}.",req.method)}));
        }
        Ok(res)
    }
}
fn nonempty(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
fn integer(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|x| i64::try_from(x).ok()))
}
fn invalid_request() -> Error {
    Error("Could not construct a valid Azure DevOps request.".into())
}
fn invalid_response() -> Error {
    Error("Azure DevOps returned a response in an unexpected format.".into())
}
fn pagination() -> Error {
    Error("Azure DevOps returned an invalid or repeating pagination cursor.".into())
}
