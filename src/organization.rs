use crate::{Error, Result, platform::is_swift_control};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use url::Url;

const ORG_ERROR: &str = "Organization must be a bare Azure DevOps organization name or an HTTPS dev.azure.com/ORG or ORG.visualstudio.com URL.";
const PR_ERROR: &str = "Pull request target must be an HTTPS Azure DevOps pull request URL.";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Organization {
    pub name: String,
}

impl Organization {
    pub fn parse(value: &str) -> Result<Self> {
        if value.is_empty() || value.trim() != value {
            return Err(Error(ORG_ERROR.into()));
        }
        let candidate = if !value.contains("://") {
            value.to_owned()
        } else {
            let url = Url::parse(value).map_err(|_| Error(ORG_ERROR.into()))?;
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
                || explicit_port(value)
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(Error(ORG_ERROR.into()));
            }
            let host = url
                .host_str()
                .ok_or_else(|| Error(ORG_ERROR.into()))?
                .to_ascii_lowercase();
            let raw_path = url.path();
            if raw_path.trim_start_matches('/').contains("//") {
                return Err(Error(ORG_ERROR.into()));
            }
            let segments: Vec<&str> = raw_path.split('/').filter(|s| !s.is_empty()).collect();
            if host == "dev.azure.com" {
                if segments.len() != 1 {
                    return Err(Error(ORG_ERROR.into()));
                }
                decode(segments[0]).map_err(|_| Error(ORG_ERROR.into()))?
            } else if let Some(org) = host.strip_suffix(".visualstudio.com") {
                if !segments.is_empty() || org.contains('.') {
                    return Err(Error(ORG_ERROR.into()));
                }
                org.to_owned()
            } else {
                return Err(Error(ORG_ERROR.into()));
            }
        };
        if !valid_name(&candidate) {
            return Err(Error(ORG_ERROR.into()));
        }
        Ok(Self {
            name: candidate.to_ascii_lowercase(),
        })
    }
    pub fn url(&self) -> String {
        format!("https://dev.azure.com/{}", self.name)
    }
}

impl Serialize for Organization {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            name: &'a str,
        }
        Wire { name: &self.name }.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Organization {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            name: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::parse(&wire.name).map_err(|_| de::Error::custom("Invalid Azure DevOps organization."))
    }
}

fn valid_name(value: &str) -> bool {
    let b = value.as_bytes();
    (1..=50).contains(&b.len())
        && b.first().is_some_and(u8::is_ascii_alphanumeric)
        && b.last().is_some_and(u8::is_ascii_alphanumeric)
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
}
fn decode(value: &str) -> std::result::Result<String, ()> {
    percent_decode_str(value)
        .decode_utf8()
        .map(|s| s.into_owned())
        .map_err(|_| ())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrLocator {
    pub organization: Organization,
    pub project: Option<String>,
    pub repository: Option<String>,
    pub id: i64,
}
impl PrLocator {
    pub fn new(
        organization: Organization,
        project: Option<String>,
        repository: Option<String>,
        id: i64,
    ) -> Self {
        Self {
            organization,
            project,
            repository,
            id,
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        if value.trim() != value {
            return Err(Error(PR_ERROR.into()));
        }
        let url = Url::parse(value).map_err(|_| Error(PR_ERROR.into()))?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || explicit_port(value)
            || url.path().trim_start_matches('/').contains("//")
        {
            return Err(Error(PR_ERROR.into()));
        }
        let host = url
            .host_str()
            .ok_or_else(|| Error(PR_ERROR.into()))?
            .to_ascii_lowercase();
        let mut segments = Vec::new();
        for raw in url.path().split('/').filter(|s| !s.is_empty()) {
            let decoded = decode(raw).map_err(|_| Error(PR_ERROR.into()))?;
            if decoded.is_empty() || decoded.contains('/') || decoded.chars().any(is_swift_control)
            {
                return Err(Error(PR_ERROR.into()));
            }
            segments.push(decoded);
        }
        let (organization, parts): (Organization, &[String]) = if host == "dev.azure.com" {
            if segments.len() != 6 {
                return Err(Error(PR_ERROR.into()));
            }
            (
                Organization::parse(&segments[0]).map_err(|_| Error(PR_ERROR.into()))?,
                &segments[1..],
            )
        } else if let Some(org) = host.strip_suffix(".visualstudio.com") {
            if org.contains('.') || !(segments.len() == 5 || segments.len() == 6) {
                return Err(Error(PR_ERROR.into()));
            }
            let org = Organization::parse(org).map_err(|_| Error(PR_ERROR.into()))?;
            if segments.len() == 6 {
                if !segments[0].eq_ignore_ascii_case("DefaultCollection") {
                    return Err(Error(PR_ERROR.into()));
                }
                (org, &segments[1..])
            } else {
                (org, &segments[..])
            }
        } else {
            return Err(Error(PR_ERROR.into()));
        };
        if parts.len() != 5
            || !parts[1].eq_ignore_ascii_case("_git")
            || !parts[3].eq_ignore_ascii_case("pullrequest")
        {
            return Err(Error(PR_ERROR.into()));
        }
        let id = parts[4]
            .parse::<i64>()
            .map_err(|_| Error(PR_ERROR.into()))?;
        if id <= 0 {
            return Err(Error(PR_ERROR.into()));
        }
        Ok(Self {
            organization,
            project: Some(parts[0].clone()),
            repository: Some(parts[2].clone()),
            id,
        })
    }
}

fn explicit_port(value: &str) -> bool {
    value
        .split_once("://")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
        .and_then(|authority| authority.rsplit('@').next())
        .is_some_and(|host| host.contains(':'))
}
