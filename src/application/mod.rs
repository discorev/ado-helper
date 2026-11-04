mod auth;
mod output;
mod pull_requests;
mod work_items;

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

    pub(crate) fn run(&self, c: CliCommand) -> Result<i32> {
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
            CliCommand::PrQuery {
                status,
                creator,
                reviewer,
                source,
                target,
                top,
                profile,
            } => self.pr_query(
                &status,
                creator.as_deref(),
                reviewer.as_deref(),
                source.as_deref(),
                target.as_deref(),
                top,
                profile.as_deref(),
            )?,
            CliCommand::PrCreate {
                source,
                target,
                title,
                description_file,
                draft,
                reviewers,
                force,
                work_items,
                tags,
                profile,
            } => self.pr_create(
                source.as_deref(),
                target.as_deref(),
                &title,
                description_file.as_deref(),
                draft,
                &reviewers,
                force,
                &work_items,
                tags.as_deref(),
                profile.as_deref(),
            )?,
            CliCommand::PrUpdate {
                target,
                title,
                description_file,
                status,
                draft,
                add_reviewers,
                add_required_reviewers,
                force,
                tags,
                target_branch,
                profile,
            } => self.pr_update(
                target,
                title.as_deref(),
                description_file.as_deref(),
                status.as_deref(),
                draft,
                &add_reviewers,
                &add_required_reviewers,
                force,
                tags.as_deref(),
                target_branch.as_deref(),
                profile.as_deref(),
            )?,
            CliCommand::PrComplete {
                target,
                merge_strategy,
                delete_source_branch,
                profile,
            } => self.pr_complete(
                target,
                &merge_strategy,
                delete_source_branch,
                profile.as_deref(),
            )?,
            CliCommand::PrChecks {
                target,
                wait,
                timeout,
                interval,
                profile,
            } => {
                return self.pr_checks(target, wait, timeout, interval, profile.as_deref());
            }
            CliCommand::PrShow { target, profile } => self.pr_show(target, profile.as_deref())?,
            CliCommand::PrOpen { target, profile } => self.pr_open(target, profile.as_deref())?,
            CliCommand::PrThreads { target, profile } => {
                self.pr_threads(target, profile.as_deref())?
            }
            CliCommand::PrVote {
                target,
                vote,
                profile,
            } => self.pr_vote(target, &vote, profile.as_deref())?,
            CliCommand::PrThreadStatus {
                target,
                thread_id,
                status,
                profile,
            } => self.pr_thread_status(target, thread_id, &status, profile.as_deref())?,
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
            CliCommand::PrEditComment {
                target,
                profile,
                thread_id,
                comment_id,
                body_file,
                expected_body_file,
            } => self.edit_comment(
                target,
                profile.as_deref(),
                thread_id,
                comment_id,
                &body_file,
                &expected_body_file,
            )?,
            CliCommand::PrDeleteComment {
                target,
                profile,
                thread_id,
                comment_id,
                expected_body_file,
            } => self.delete_comment(
                target,
                profile.as_deref(),
                thread_id,
                comment_id,
                &expected_body_file,
            )?,
            CliCommand::PrReplyComment {
                target,
                profile,
                thread_id,
                parent_comment_id,
                body_file,
            } => self.reply_comment(
                target,
                profile.as_deref(),
                thread_id,
                parent_comment_id,
                &body_file,
            )?,
            CliCommand::PrTopLevelComment {
                target,
                profile,
                body_file,
                commit,
            } => self.top_level_comment(target, profile.as_deref(), &body_file, &commit)?,
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
            CliCommand::WiQuery {
                query,
                top,
                profile,
                project,
            } => self.wi_query(query, top, profile.as_deref(), project.as_deref())?,
            CliCommand::WiShow {
                id,
                profile,
                project,
            } => self.wi_show(id, profile.as_deref(), project.as_deref())?,
            CliCommand::WiComments {
                id,
                profile,
                project,
            } => self.wi_comments(id, profile.as_deref(), project.as_deref())?,
            CliCommand::WiComment {
                id,
                body_file,
                profile,
                project,
            } => self.wi_comment(id, &body_file, profile.as_deref(), project.as_deref())?,
            CliCommand::WiCreate {
                work_item_type,
                title,
                description_file,
                assign_to,
                tags,
                profile,
                project,
            } => self.wi_create(
                &work_item_type,
                &title,
                description_file.as_deref(),
                assign_to.as_deref(),
                tags.as_deref(),
                profile.as_deref(),
                project.as_deref(),
            )?,
            CliCommand::WiUpdate {
                id,
                state,
                title,
                assign_to,
                tags,
                description_file,
                profile,
                project,
            } => self.wi_update(
                id,
                state.as_deref(),
                title.as_deref(),
                assign_to.as_deref(),
                tags.as_deref(),
                description_file.as_deref(),
                profile.as_deref(),
                project.as_deref(),
            )?,
            CliCommand::WiOpen {
                id,
                profile,
                project,
            } => self.wi_open(id, profile.as_deref(), project.as_deref())?,
        }
        Ok(0)
    }
}
