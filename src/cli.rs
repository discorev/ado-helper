use crate::{Error, Result, git::is_commit_sha};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestTarget {
    CurrentBranch,
    Number(i64),
    Url(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    AuthAdd {
        name: String,
        organization: String,
        open_browser: bool,
    },
    AuthUpdate {
        name: String,
        open_browser: bool,
    },
    AuthStatus {
        check: bool,
    },
    AuthRemove {
        name: String,
    },
    PrShow {
        target: PullRequestTarget,
        profile: Option<String>,
    },
    PrThreads {
        target: PullRequestTarget,
        profile: Option<String>,
    },
    PrChanges {
        target: PullRequestTarget,
        profile: Option<String>,
        iteration: Option<i64>,
    },
    PrClone {
        target: PullRequestTarget,
        profile: Option<String>,
        directory: Option<String>,
    },
    PrDiff {
        target: PullRequestTarget,
        profile: Option<String>,
        directory: Option<String>,
    },
    PrComment {
        target: PullRequestTarget,
        profile: Option<String>,
        file: String,
        line: i64,
        end_line: i64,
        side: String,
        body_file: String,
        commit: String,
        iteration: i64,
        change_id: i64,
    },
}

pub fn parse(arguments: &[String]) -> Result<CliCommand> {
    let Some(group) = arguments.first() else {
        return Ok(CliCommand::Help);
    };
    if ["help", "--help", "-h"].contains(&group.as_str()) {
        return Ok(CliCommand::Help);
    }
    if arguments.len() < 2 {
        return Err(Error("Missing command. Run 'ado help' for usage.".into()));
    }
    let args = &arguments[2..];
    match (group.as_str(), arguments[1].as_str()) {
        ("auth", "add") => parse_auth_add(args),
        ("auth", "update") => parse_auth_update(args),
        ("auth", "status") => parse_auth_status(args),
        ("auth", "remove") => parse_auth_remove(args),
        ("pr", "show") => parse_simple(args, true),
        ("pr", "threads") => parse_simple(args, false),
        ("pr", "changes") => parse_changes(args),
        ("pr", "clone") => parse_checkout(args, true),
        ("pr", "diff") => parse_checkout(args, false),
        ("pr", "comment") => parse_comment(args),
        _ => Err(Error(format!(
            "Unknown command '{} {}'. Run 'ado help' for usage.",
            group, arguments[1]
        ))),
    }
}
fn positional<'a>(args: &'a [String], label: &str) -> Result<(&'a str, usize)> {
    match args.first() {
        Some(v) if !v.starts_with('-') => Ok((v, 1)),
        _ => Err(Error(format!("Missing {label}."))),
    }
}
fn parse_auth_add(args: &[String]) -> Result<CliCommand> {
    let (name, mut i) = positional(args, "profile name")?;
    let mut org = None;
    let mut browser = true;
    let mut no_browser = false;
    while i < args.len() {
        match args[i].as_str() {
            "--org" => {
                if org.is_some() {
                    return Err(Error("Option '--org' may only be specified once.".into()));
                }
                let Some(v) = args.get(i + 1).filter(|v: &&String| !v.starts_with("--")) else {
                    return Err(Error("Option '--org' requires a value.".into()));
                };
                org = Some(v.clone());
                i += 2;
            }
            "--no-browser" => {
                if no_browser {
                    return Err(Error(
                        "Option '--no-browser' may only be specified once.".into(),
                    ));
                }
                no_browser = true;
                browser = false;
                i += 1;
            }
            v => return Err(Error(format!("Unexpected argument '{v}'."))),
        }
    }
    Ok(CliCommand::AuthAdd {
        name: name.into(),
        organization: org.ok_or_else(|| Error("auth add requires --org URL.".into()))?,
        open_browser: browser,
    })
}
fn parse_auth_update(args: &[String]) -> Result<CliCommand> {
    let (name, mut i) = positional(args, "profile name")?;
    let mut browser = true;
    let mut seen = false;
    while i < args.len() {
        if args[i] != "--no-browser" {
            return Err(Error(format!("Unexpected argument '{}'.", args[i])));
        }
        if seen {
            return Err(Error(
                "Option '--no-browser' may only be specified once.".into(),
            ));
        }
        seen = true;
        browser = false;
        i += 1;
    }
    Ok(CliCommand::AuthUpdate {
        name: name.into(),
        open_browser: browser,
    })
}
fn parse_auth_status(args: &[String]) -> Result<CliCommand> {
    if args.len() > 1 || args.first().is_some_and(|v| v != "--check") {
        return Err(Error("Usage: ado auth status [--check]".into()));
    }
    Ok(CliCommand::AuthStatus {
        check: !args.is_empty(),
    })
}
fn parse_auth_remove(args: &[String]) -> Result<CliCommand> {
    if args.len() != 1 || args[0].starts_with('-') {
        return Err(Error("Usage: ado auth remove NAME".into()));
    }
    Ok(CliCommand::AuthRemove {
        name: args[0].clone(),
    })
}
struct Values {
    positional: Option<String>,
    options: BTreeMap<String, String>,
}
fn common(args: &[String], allowed: &[&str]) -> Result<Values> {
    let allowed: BTreeSet<&str> = allowed.iter().copied().collect();
    let mut r = Values {
        positional: None,
        options: BTreeMap::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let v = &args[i];
        if v.starts_with('-') {
            if !allowed.contains(v.as_str()) {
                return Err(Error(format!("Unexpected option '{v}'.")));
            }
            if r.options.contains_key(v) {
                return Err(Error(format!("Option '{v}' may only be specified once.")));
            }
            let Some(next) = args.get(i + 1).filter(|n| !n.starts_with("--")) else {
                return Err(Error(format!("Option '{v}' requires a value.")));
            };
            r.options.insert(v.clone(), next.clone());
            i += 2;
        } else {
            if r.positional.is_some() {
                return Err(Error(
                    "Only one pull request target may be specified.".into(),
                ));
            }
            r.positional = Some(v.clone());
            i += 1;
        }
    }
    Ok(r)
}
impl Values {
    fn target(&mut self) -> Result<PullRequestTarget> {
        match self.positional.take() {
            None => Ok(PullRequestTarget::CurrentBranch),
            Some(v) if v.to_ascii_lowercase().starts_with("https://") => {
                Ok(PullRequestTarget::Url(v))
            }
            Some(v) => {
                let id=v.parse::<i64>().map_err(|_|Error("TARGET must be an Azure DevOps pull request URL or a positive pull request number.".into()))?;
                if id <= 0 {
                    Err(Error("TARGET must be an Azure DevOps pull request URL or a positive pull request number.".into()))
                } else {
                    Ok(PullRequestTarget::Number(id))
                }
            }
        }
    }
    fn take(&mut self, n: &str) -> Option<String> {
        self.options.remove(n)
    }
    fn required(&mut self, n: &str) -> Result<String> {
        self.take(n)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error(format!("pr comment requires {n} VALUE.")))
    }
    fn positive(&mut self, n: &str) -> Result<Option<i64>> {
        let Some(v) = self.take(n) else {
            return Ok(None);
        };
        let x = v
            .parse::<i64>()
            .map_err(|_| Error(format!("{n} must be a positive integer.")))?;
        if x <= 0 {
            Err(Error(format!("{n} must be a positive integer.")))
        } else {
            Ok(Some(x))
        }
    }
    fn required_positive(&mut self, n: &str) -> Result<i64> {
        self.positive(n)?
            .ok_or_else(|| Error(format!("pr comment requires {n} N.")))
    }
}
fn parse_simple(args: &[String], show: bool) -> Result<CliCommand> {
    let mut v = common(args, &["--profile"])?;
    let target = v.target()?;
    let profile = v.take("--profile");
    if show {
        Ok(CliCommand::PrShow { target, profile })
    } else {
        Ok(CliCommand::PrThreads { target, profile })
    }
}
fn parse_changes(args: &[String]) -> Result<CliCommand> {
    let mut v = common(args, &["--profile", "--iteration"])?;
    let target = v.target()?;
    let profile = v.take("--profile");
    let iteration = v.positive("--iteration")?;
    Ok(CliCommand::PrChanges {
        target,
        profile,
        iteration,
    })
}
fn parse_checkout(args: &[String], clone: bool) -> Result<CliCommand> {
    let mut v = common(args, &["--profile", "--directory"])?;
    let target = v.target()?;
    let profile = v.take("--profile");
    let directory = v.take("--directory");
    if clone {
        Ok(CliCommand::PrClone {
            target,
            profile,
            directory,
        })
    } else {
        Ok(CliCommand::PrDiff {
            target,
            profile,
            directory,
        })
    }
}
fn parse_comment(args: &[String]) -> Result<CliCommand> {
    let mut v = common(
        args,
        &[
            "--profile",
            "--file",
            "--line",
            "--end-line",
            "--side",
            "--body-file",
            "--commit",
            "--iteration",
            "--change-id",
        ],
    )?;
    let target = v.target()?;
    let profile = v.take("--profile");
    let file = v.required("--file")?;
    let line = v.required_positive("--line")?;
    let end_line = v.positive("--end-line")?.unwrap_or(line);
    if end_line < line {
        return Err(Error(
            "--end-line must be greater than or equal to --line.".into(),
        ));
    }
    let side = v.required("--side")?;
    if side != "left" && side != "right" {
        return Err(Error("--side must be 'left' or 'right'.".into()));
    }
    let body_file = v.required("--body-file")?;
    let commit = v.required("--commit")?;
    if !is_commit_sha(&commit) {
        return Err(Error(
            "--commit must be a 40-character hexadecimal commit SHA.".into(),
        ));
    }
    let iteration = v.required_positive("--iteration")?;
    let change_id = v.required_positive("--change-id")?;
    Ok(CliCommand::PrComment {
        target,
        profile,
        file,
        line,
        end_line,
        side,
        body_file,
        commit,
        iteration,
        change_id,
    })
}
