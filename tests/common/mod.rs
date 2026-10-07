#![allow(dead_code, unused_imports)]

pub use ado_core::{
    Error, Result,
    ado_client::{AdoClient, HttpRequest, HttpResponse, HttpTransport},
    auth::{self, AuthOps},
    cli::{self, CliCommand, PullRequestTarget},
    comment_anchor,
    git::*,
    keychain::{CredentialStore, KeychainStore},
    models::AdoIdentity,
    organization::{Organization, PrLocator},
    profiles::{Profile, ProfileStore},
};
pub use serde_json::{Value, json};
pub use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
pub use tempfile::TempDir;

pub fn profile(name: &str, org: &str, id: &str) -> Profile {
    Profile {
        name: name.into(),
        organization: Organization::parse(org).unwrap(),
        identity: AdoIdentity {
            id: id.into(),
            display_name: "Test User".into(),
            unique_name: "test@example.com".into(),
        },
    }
}

pub fn svec(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

type Handler = Box<dyn Fn(&HttpRequest, usize) -> Result<HttpResponse> + Send + Sync>;

pub struct Mock {
    count: Mutex<usize>,
    handler: Handler,
}

impl Mock {
    pub fn new(
        f: impl Fn(&HttpRequest, usize) -> Result<HttpResponse> + Send + Sync + 'static,
    ) -> Self {
        Self {
            count: Mutex::new(0),
            handler: Box::new(f),
        }
    }

    pub fn count(&self) -> usize {
        *self.count.lock().unwrap()
    }
}

impl HttpTransport for Mock {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse> {
        let mut count = self.count.lock().unwrap();
        let index = *count;
        *count += 1;
        drop(count);
        (self.handler)(request, index)
    }
}

pub fn response(
    request: &HttpRequest,
    status: u16,
    json: &Value,
    headers: &[(&str, &str)],
) -> HttpResponse {
    HttpResponse {
        status,
        url: request.url.clone(),
        headers: headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        body: serde_json::to_vec(json).unwrap(),
    }
}

pub fn query(request: &HttpRequest) -> HashMap<String, String> {
    request
        .url
        .query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}
