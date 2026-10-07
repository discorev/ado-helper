mod auth;
mod output;
mod pull_requests;

use crate::HELP;
use ado_core::{Result, cli::CliCommand, keychain::KeychainStore, profiles::ProfileStore};

pub(crate) struct Application {
    store: ProfileStore,
    keychain: KeychainStore,
}

impl Application {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self {
            store: ProfileStore::new(None)?,
            keychain: KeychainStore,
        })
    }

    pub(crate) fn run(&self, c: CliCommand) -> Result<()> {
        match c {
            CliCommand::Help => println!("{HELP}"),
            CliCommand::AuthAdd {
                name,
                organization,
                open_browser,
            } => self.auth_add(&name, &organization, open_browser)?,
            CliCommand::AuthUpdate { name, open_browser } => {
                self.auth_update(&name, open_browser)?
            }
            CliCommand::AuthStatus { check } => self.auth_status(check)?,
            CliCommand::AuthRemove { name } => self.auth_remove(&name)?,
            CliCommand::PrShow { target, profile } => self.pr_show(target, profile.as_deref())?,
            CliCommand::PrThreads { target, profile } => {
                self.pr_threads(target, profile.as_deref())?
            }
            CliCommand::PrChanges {
                target,
                profile,
                iteration,
            } => self.pr_changes(target, profile.as_deref(), iteration)?,
            CliCommand::PrClone {
                target,
                profile,
                directory,
            } => self.pr_clone(target, profile.as_deref(), directory)?,
            CliCommand::PrDiff {
                target,
                profile,
                directory,
            } => self.pr_diff(target, profile.as_deref(), directory)?,
            CliCommand::PrComment {
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
            } => self.comment(
                target,
                profile.as_deref(),
                &file,
                line,
                end_line,
                &side,
                &body_file,
                &commit,
                iteration,
                change_id,
            )?,
        }
        Ok(())
    }
}
