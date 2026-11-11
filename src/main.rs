mod application;

use ado_core::{Error, Result, cli::CliCommand, platform::terminal_safe};
use application::Application;

fn main() {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    match run() {
        Err(error) => {
            eprintln!("error: {}", terminal_safe(&error.to_string()));
            std::process::exit(1)
        }
        Ok(code) if code != 0 => std::process::exit(code),
        Ok(_) => {}
    }
}

fn run() -> Result<i32> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| Error("Command-line arguments must be valid UTF-8.".into()))
        })
        .collect::<Result<_>>()?;
    // Checked before parsing so `--help` never runs the command it follows.
    if let Some(help) = command_help(&args) {
        println!("{help}");
        return Ok(0);
    }
    let command = ado_core::cli::parse(&args)?;
    if command == CliCommand::Help {
        println!("{HELP}");
        return Ok(0);
    }
    Application::new()?.run(command)
}

const HELP: &str = "Usage:
  ado auth add NAME --org URL [--no-browser]
  ado auth update NAME [--no-browser]
  ado auth status [--check]
  ado auth remove NAME
  ado pr query [--status active|completed|abandoned|all] [--creator me|USER]
      [--reviewer me|USER] [--source BRANCH] [--target BRANCH] [--top N] [--profile NAME]
  ado pr show [TARGET] [--profile NAME]
  ado pr create [--source BRANCH] [--target BRANCH] --title TEXT
      [--description-file PATH] [--draft] [--reviewer USER]... [--force] [--work-item ID]...
      [--tags TAGS] [--auto-complete [--merge-strategy noFastForward|squash|rebase|rebaseMerge]
      [--delete-source-branch | --keep-source-branch]] [--profile NAME]
  ado pr update [TARGET] [--title TEXT] [--description-file PATH]
      [--status active|abandoned] [--draft | --ready] [--add-reviewer USER]... [--force]
      [--add-required-reviewer USER]... [--tags TAGS] [--target BRANCH]
      [--add-work-item ID]... [--profile NAME]
  ado pr complete [TARGET] [--merge-strategy no-fast-forward|squash|rebase|rebase-merge]
      [--delete-source-branch] [--profile NAME]
  ado pr auto-complete [TARGET] [--merge-strategy noFastForward|squash|rebase|rebaseMerge]
      [--delete-source-branch | --keep-source-branch] [--profile NAME]
  ado pr auto-complete [TARGET] --clear [--profile NAME]
  ado pr checks [TARGET] [--wait] [--timeout SECONDS] [--interval SECONDS] [--profile NAME]
  ado pr open TARGET [--profile NAME]
  ado pr threads [TARGET] [--profile NAME]
  ado pr vote [TARGET] --vote approve|approve-with-suggestions|wait|reject|reset
      [--profile NAME]
  ado pr thread-status [TARGET] --thread-id N
      --status active|fixed|wontfix|closed|bydesign|pending [--profile NAME]
  ado pr changes [TARGET] [--profile NAME] [--iteration N]
  ado pr clone [TARGET] [--profile NAME] [--directory PATH]
  ado pr diff [TARGET] [--profile NAME] [--directory PATH]
  ado pr edit-comment [TARGET] --thread-id N --comment-id N --body-file PATH
      --expected-body-file PATH [--profile NAME]
  ado pr delete-comment [TARGET] --thread-id N --comment-id N
      --expected-body-file PATH [--profile NAME]
  ado pr comment [TARGET] --thread-id N [--parent-comment-id N] --body-file PATH
      [--profile NAME]
  ado pr comment [TARGET] --top-level --body-file PATH --commit SHA [--profile NAME]
  ado pr comment [TARGET] --file PATH --line N [--end-line N] --side left|right
      --body-file PATH --commit SHA --iteration N --change-id N [--profile NAME]
  ado wi query (--wiql QUERY | --wiql-file PATH | --search TEXT) [--top N]
      [--project NAME] [--profile NAME]
  ado wi show ID [--project NAME] [--profile NAME]
  ado wi comments ID [--project NAME] [--profile NAME]
  ado wi comment ID --body-file PATH [--project NAME] [--profile NAME]
  ado wi create --type TYPE --title TEXT [--description-file PATH] [--assign-to USER]
      [--tags TAGS] [--priority 1-4] [--area PATH] [--iteration PATH]
      [--field REF=VALUE]... [--project NAME] [--profile NAME]
  ado wi update ID [--state STATE] [--title TEXT] [--assign-to USER] [--tags TAGS]
      [--description-file PATH] [--area PATH] [--iteration PATH]
      [--field REF=VALUE]... [--project NAME] [--profile NAME]
  ado wi open ID [--project NAME] [--profile NAME]
  ado wi fields --type TYPE [--project NAME] [--profile NAME]

TARGET is an Azure DevOps PR URL or positive PR number. Omit it where allowed to use the current branch.
Work-item commands derive the project from the current Azure Git remote unless --project is supplied.
Command data is written as JSON; open commands print the URL before launching it.
--field sets any field by reference name (e.g. Custom.Severity=High); wi fields --type TYPE lists reference names and allowed values.";

/// Usage for `ado GROUP [COMMAND] ... --help`: the matching HELP entries plus its notes.
/// Returns None for an unknown group or command so the parser reports it as usual.
fn command_help(args: &[String]) -> Option<String> {
    let at = args
        .iter()
        .position(|arg| arg == "--help" || arg == "-h")
        .filter(|&at| at > 0)?;
    let prefix = format!("  ado {}", args[..at.min(2)].join(" "));
    let (usage, notes) = HELP.split_once("\n\n")?;
    let mut entries = Vec::new();
    let mut matching = false;
    for line in usage.lines().skip(1) {
        if line.starts_with("  ado ") {
            matching = line
                .strip_prefix(&prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '));
        }
        if matching {
            entries.push(line);
        }
    }
    (!entries.is_empty()).then(|| format!("Usage:\n{}\n\n{notes}", entries.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::command_help;

    fn help(args: &[&str]) -> Option<String> {
        command_help(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn command_help_shows_only_the_matching_entries() {
        let text = help(&["pr", "auto-complete", "62039", "--help"]).unwrap();
        assert!(text.starts_with("Usage:\n  ado pr auto-complete [TARGET] [--merge-strategy"));
        assert!(text.contains("      [--delete-source-branch | --keep-source-branch]"));
        assert!(text.contains("  ado pr auto-complete [TARGET] --clear [--profile NAME]"));
        assert!(text.contains("TARGET is an Azure DevOps PR URL"));
        assert!(!text.contains("ado pr complete"));
        let checks = help(&["pr", "checks", "--help"]).unwrap();
        assert!(checks.starts_with("Usage:\n  ado pr checks [TARGET] [--wait]"));
        assert!(checks.contains("[--timeout SECONDS] [--interval SECONDS] [--profile NAME]"));
        assert!(!checks.contains("ado pr auto-complete"));
        assert_eq!(help(&["pr", "checks", "-h"]), Some(checks));
        let comment = help(&["pr", "comment", "-h"]).unwrap();
        assert_eq!(comment.matches("  ado pr comment ").count(), 3);
        assert!(!comment.contains("edit-comment"));
        let wi = help(&["wi", "comment", "-h"]).unwrap();
        assert!(!wi.contains("ado wi comments"));
    }

    #[test]
    fn group_help_lists_the_group_and_unknown_commands_fall_through() {
        let group = help(&["pr", "--help"]).unwrap();
        assert!(group.contains("ado pr create") && !group.contains("ado wi"));
        assert!(group.contains("ado pr checks [TARGET]"));
        assert!(group.contains("ado pr auto-complete [TARGET]"));
        assert_eq!(help(&["pr", "nope", "--help"]), None);
        assert_eq!(help(&["--help"]), None);
        assert_eq!(help(&["pr", "show", "1"]), None);
    }
}
