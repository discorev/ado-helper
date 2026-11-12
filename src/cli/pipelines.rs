use super::{CliCommand, multi_values};
use crate::{
    Error, Result, git::validate_ref, organization::Organization, platform::is_swift_control,
};
use percent_encoding::percent_decode_str;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineTarget {
    Id(i64),
    Url {
        organization: Organization,
        project: String,
        id: i64,
    },
}

impl PipelineTarget {
    pub fn id(&self) -> i64 {
        match self {
            Self::Id(id) | Self::Url { id, .. } => *id,
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        let invalid = || {
            Error("RUN must be a positive build ID or an HTTPS dev.azure.com/ORG/PROJECT/_build/results?buildId=N URL.".into())
        };
        if !value.contains("://") {
            return value
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .map(Self::Id)
                .ok_or_else(invalid);
        }
        let url = Url::parse(value).map_err(|_| invalid())?;
        let authority = value
            .split_once("://")
            .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default());
        if value.trim() != value
            || url.scheme() != "https"
            || url.host_str() != Some("dev.azure.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || authority.is_some_and(|host| host.contains(':'))
        {
            return Err(invalid());
        }
        let parts = url
            .path()
            .strip_prefix('/')
            .ok_or_else(invalid)?
            .split('/')
            .map(|part| {
                let decoded = percent_decode_str(part)
                    .decode_utf8()
                    .map_err(|_| invalid())?;
                if decoded.is_empty()
                    || decoded.contains('/')
                    || decoded.contains('\\')
                    || decoded == "."
                    || decoded == ".."
                    || decoded.chars().any(is_swift_control)
                {
                    return Err(invalid());
                }
                Ok(decoded.into_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        if parts.len() != 4 || parts[2] != "_build" || parts[3] != "results" {
            return Err(invalid());
        }
        let ids = url
            .query_pairs()
            .filter(|(key, _)| key == "buildId")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        let [id] = ids.as_slice() else {
            return Err(invalid());
        };
        let id = id
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(invalid)?;
        Ok(Self::Url {
            organization: Organization::parse(&parts[0]).map_err(|_| invalid())?,
            project: parts[1].clone(),
            id,
        })
    }
}

pub(super) fn parse(args: &[String], command: &str) -> Result<CliCommand> {
    let (options, flags): (&[&str], &[&str]) = match command {
        "list" => (&["--name"], &[]),
        "runs" => (
            &["--pipeline", "--branch", "--status", "--result", "--top"],
            &[],
        ),
        "show" | "open" => (&[], &[]),
        "log" => (&["--log-id", "--tail"], &["--failed"]),
        "tests" => (&[], &["--all"]),
        _ => {
            return Err(Error(format!(
                "Unknown command 'pipeline {command}'. Run 'ado help' for usage."
            )));
        }
    };
    let mut allowed = vec!["--profile", "--project"];
    allowed.extend_from_slice(options);
    let mut v = multi_values(args, &allowed, &[], flags)?;
    let profile = v.take("--profile");
    let project = v.take("--project");
    for (flag, value) in [("--profile", &profile), ("--project", &project)] {
        if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
            return Err(Error(format!("{flag} must not be empty.")));
        }
    }
    if command == "list" || command == "runs" {
        if v.positional.is_some() {
            return Err(Error(format!(
                "pipeline {command} does not accept a positional argument."
            )));
        }
        if command == "list" {
            return Ok(CliCommand::PipelineList {
                name: v.take("--name"),
                profile,
                project,
            });
        }
        let pipeline = v.take("--pipeline");
        if let Some(value) = &pipeline
            && (value.trim().is_empty() || value.parse::<i64>().is_ok_and(|id| id <= 0))
        {
            return Err(Error(
                "--pipeline must be a positive definition ID or a nonempty name.".into(),
            ));
        }
        let branch = v
            .take("--branch")
            .map(|branch| -> Result<String> {
                let branch = if branch.starts_with("refs/heads/") {
                    branch
                } else {
                    format!("refs/heads/{branch}")
                };
                validate_ref(&branch)?;
                Ok(branch)
            })
            .transpose()?;
        return Ok(CliCommand::PipelineRuns {
            pipeline,
            branch,
            status: enum_option(
                v.take("--status"),
                "--status",
                &[
                    "none",
                    "inProgress",
                    "completed",
                    "cancelling",
                    "postponed",
                    "notStarted",
                    "all",
                ],
            )?,
            result: enum_option(
                v.take("--result"),
                "--result",
                &[
                    "none",
                    "succeeded",
                    "partiallySucceeded",
                    "failed",
                    "canceled",
                ],
            )?,
            top: v.positive("--top")?.unwrap_or(25),
            profile,
            project,
        });
    }
    let raw = v
        .positional
        .take()
        .ok_or_else(|| Error(format!("pipeline {command} requires RUN.")))?;
    let target = PipelineTarget::parse(&raw)?;
    Ok(match command {
        "show" => CliCommand::PipelineShow {
            target,
            profile,
            project,
        },
        "open" => CliCommand::PipelineOpen {
            target,
            profile,
            project,
        },
        "tests" => CliCommand::PipelineTests {
            target,
            all: v.flags.contains("--all"),
            profile,
            project,
        },
        "log" => {
            let log_id = v.positive("--log-id")?;
            let failed = v.flags.contains("--failed");
            if log_id.is_some() == failed {
                return Err(Error(
                    "pipeline log requires exactly one of --log-id N or --failed.".into(),
                ));
            }
            let tail = v
                .take("--tail")
                .map(|value| {
                    value
                        .parse::<usize>()
                        .map_err(|_| Error("--tail must be a non-negative integer.".into()))
                })
                .transpose()?;
            CliCommand::PipelineLog {
                target,
                log_id,
                failed,
                tail,
                profile,
                project,
            }
        }
        _ => unreachable!(),
    })
}

fn enum_option(value: Option<String>, flag: &str, allowed: &[&str]) -> Result<Option<String>> {
    value
        .map(|value| {
            allowed
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(&value))
                .map(|candidate| (*candidate).to_owned())
                .ok_or_else(|| Error(format!("{flag} must be one of: {}.", allowed.join(", "))))
        })
        .transpose()
}
