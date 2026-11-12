use super::{
    Application,
    output::{integer, open_browser, write_json},
    work_items::Scope,
};
use ado_core::{
    Error, Result,
    ado_client::{AdoClient, HttpTransport, builds::BuildQuery},
    cli::PipelineTarget,
    keychain::CredentialStore,
    organization::Organization,
    platform::terminal_safe,
};
use serde_json::{Value, json};
use std::io::{self, Write};
use url::Url;

mod output;
use output::{
    definition_output, failed_task_logs, run_output, tail_lines, test_run_output, timeline_output,
};

impl Application {
    pub(super) fn pipeline_list(
        &self,
        name: Option<&str>,
        profile: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.work_item_scope(profile, project)?;
        let definitions = self
            .pipeline_client(&scope)?
            .build_definitions(&scope.project, name)?;
        write_json(&Value::Array(
            definitions
                .iter()
                .map(|definition| {
                    definition_output(definition, &scope.organization, &scope.project)
                })
                .collect::<Result<_>>()?,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn pipeline_runs(
        &self,
        pipeline: Option<&str>,
        branch: Option<&str>,
        status: Option<&str>,
        result: Option<&str>,
        top: i64,
        profile: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.work_item_scope(profile, project)?;
        let client = self.pipeline_client(&scope)?;
        let definition = pipeline
            .map(|pipeline| resolve_definition(&client, &scope.project, pipeline))
            .transpose()?;
        let builds = client.builds(
            &scope.project,
            &BuildQuery {
                definition,
                branch,
                status,
                result,
                top,
            },
        )?;
        write_json(&Value::Array(
            builds
                .iter()
                .map(|build| run_output(build, &scope.organization, &scope.project))
                .collect::<Result<_>>()?,
        ))
    }

    pub(super) fn pipeline_show(
        &self,
        target: PipelineTarget,
        profile: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.pipeline_scope(&target, profile, project)?;
        let client = self.pipeline_client(&scope)?;
        let id = target.id();
        let mut output = run_output(
            &client.build(&scope.project, id)?,
            &scope.organization,
            &scope.project,
        )?;
        output["timeline"] = timeline_output(&client.build_timeline(&scope.project, id)?)?;
        let tests = client.build_test_runs(&scope.project, id)?;
        output["tests"] = json!({"exists": !tests.is_empty(), "count": tests.len()});
        write_json(&output)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn pipeline_log(
        &self,
        target: PipelineTarget,
        log_id: Option<i64>,
        failed: bool,
        tail: Option<usize>,
        profile: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.pipeline_scope(&target, profile, project)?;
        let client = self.pipeline_client(&scope)?;
        let id = target.id();
        if let Some(log_id) = log_id {
            let text = client.build_log(&scope.project, id, log_id)?;
            io::stdout()
                .lock()
                .write_all(tail_lines(&text, tail).as_bytes())?;
        } else if failed {
            let records = client.build_timeline(&scope.project, id)?;
            let logs = failed_task_logs(&records)?;
            if logs.is_empty() {
                eprintln!("Build {id} has no failed tasks.");
            }
            for (log_id, name) in logs {
                let mut stdout = io::stdout().lock();
                let Some(log_id) = log_id else {
                    writeln!(
                        stdout,
                        "=== {} (no log available) ===",
                        terminal_safe(&name)
                    )?;
                    continue;
                };
                let text = client.build_log(&scope.project, id, log_id)?;
                writeln!(stdout, "=== {} (log {log_id}) ===", terminal_safe(&name))?;
                let text = tail_lines(&text, tail);
                stdout.write_all(text.as_bytes())?;
                if !text.ends_with('\n') {
                    writeln!(stdout)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn pipeline_tests(
        &self,
        target: PipelineTarget,
        all: bool,
        profile: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.pipeline_scope(&target, profile, project)?;
        let client = self.pipeline_client(&scope)?;
        let mut output = Vec::new();
        for run in client.build_test_runs(&scope.project, target.id())? {
            let run_id = positive_id(&run)?;
            let statistics = client.test_run_statistics(&scope.project, run_id)?;
            let results = client.test_results(&scope.project, run_id, all)?;
            output.push(test_run_output(&run, &statistics, &results)?);
        }
        write_json(&json!({"buildId": target.id(), "runs": output}))
    }

    pub(super) fn pipeline_open(
        &self,
        target: PipelineTarget,
        profile: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        let scope = self.pipeline_scope(&target, profile, project)?;
        let url = build_url(&scope.organization, &scope.project, target.id())?;
        println!("{url}");
        open_browser(&url)
    }

    fn pipeline_scope(
        &self,
        target: &PipelineTarget,
        profile_name: Option<&str>,
        project_override: Option<&str>,
    ) -> Result<Scope> {
        let PipelineTarget::Url {
            organization,
            project,
            ..
        } = target
        else {
            return self.work_item_scope(profile_name, project_override);
        };
        if project_override.is_some_and(|value| !value.eq_ignore_ascii_case(project)) {
            return Err(Error(
                "--project conflicts with the project in the build URL.".into(),
            ));
        }
        let profile = match profile_name {
            Some(name) => self.store.profile_by_name(name)?,
            None => self.store.profile_by_organization(organization)?,
        };
        if profile.organization != *organization {
            return Err(Error(format!(
                "Profile '{}' belongs to {}, not {}.",
                profile.name, profile.organization.name, organization.name
            )));
        }
        Ok(Scope {
            organization: organization.clone(),
            project: project.clone(),
            profile,
        })
    }

    fn pipeline_client(&self, scope: &Scope) -> Result<AdoClient> {
        AdoClient::new(
            scope.organization.clone(),
            &self.keychain.read(&scope.profile)?,
        )
    }
}

fn resolve_definition<T: HttpTransport>(
    client: &AdoClient<T>,
    project: &str,
    pipeline: &str,
) -> Result<i64> {
    if let Ok(id) = pipeline.parse::<i64>() {
        return if id > 0 {
            Ok(id)
        } else {
            Err(Error("Pipeline definition ID must be positive.".into()))
        };
    }
    let matches = client
        .build_definitions(project, Some(pipeline))?
        .into_iter()
        .filter(|definition| {
            definition
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case(pipeline))
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [definition] => positive_id(definition),
        [] => Err(Error(format!(
            "No pipeline named '{pipeline}' was found in project '{project}'."
        ))),
        _ => Err(Error(format!(
            "Pipeline name '{pipeline}' matches {} definitions; pass a definition ID from 'ado pipeline list'.",
            matches.len()
        ))),
    }
}

fn positive_id(value: &Value) -> Result<i64> {
    value
        .get("id")
        .and_then(integer)
        .filter(|id| *id > 0)
        .ok_or_else(|| Error("Azure DevOps response is missing a positive ID.".into()))
}

fn build_url(organization: &Organization, project: &str, id: i64) -> Result<String> {
    let mut url =
        Url::parse(&organization.url()).map_err(|_| Error("Invalid organization URL.".into()))?;
    url.path_segments_mut()
        .map_err(|_| Error("Invalid build URL.".into()))?
        .extend([project, "_build", "results"]);
    url.query_pairs_mut()
        .append_pair("buildId", &id.to_string());
    Ok(url.into())
}

#[cfg(test)]
mod tests;
