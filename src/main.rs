mod application;

use ado_core::{Error, Result, cli::CliCommand, platform::terminal_safe};
use application::Application;

fn main() {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    if let Err(error) = run() {
        eprintln!("error: {}", terminal_safe(&error.to_string()));
        std::process::exit(1)
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| Error("Command-line arguments must be valid UTF-8.".into()))
        })
        .collect::<Result<_>>()?;
    let command = ado_core::cli::parse(&args)?;
    if command == CliCommand::Help {
        println!("{HELP}");
        return Ok(());
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
      [--description-file PATH] [--draft] [--reviewer USER]... [--work-item ID]...
      [--tags TAGS] [--profile NAME]
  ado pr update [TARGET] [--title TEXT] [--description-file PATH]
      [--status active|abandoned] [--draft | --ready] [--add-reviewer USER]...
      [--tags TAGS] [--profile NAME]
  ado pr open TARGET [--profile NAME]
  ado pr threads [TARGET] [--profile NAME]
  ado pr changes [TARGET] [--profile NAME] [--iteration N]
  ado pr clone [TARGET] [--profile NAME] [--directory PATH]
  ado pr diff [TARGET] [--profile NAME] [--directory PATH]
  ado pr edit-comment [TARGET] --thread-id N --comment-id N --body-file PATH
      --expected-body-file PATH [--profile NAME]
  ado pr comment [TARGET] --top-level --body-file PATH --commit SHA [--profile NAME]
  ado pr comment [TARGET] --file PATH --line N [--end-line N] --side left|right
      --body-file PATH --commit SHA --iteration N --change-id N [--profile NAME]
  ado wi query (--wiql QUERY | --wiql-file PATH | --search TEXT) [--top N]
      [--project NAME] [--profile NAME]
  ado wi show ID [--project NAME] [--profile NAME]
  ado wi comments ID [--project NAME] [--profile NAME]
  ado wi comment ID --body-file PATH [--project NAME] [--profile NAME]
  ado wi create --type TYPE --title TEXT [--description-file PATH] [--assign-to USER]
      [--tags TAGS] [--project NAME] [--profile NAME]
  ado wi update ID [--state STATE] [--title TEXT] [--assign-to USER] [--tags TAGS]
      [--description-file PATH] [--project NAME] [--profile NAME]
  ado wi open ID [--project NAME] [--profile NAME]

TARGET is an Azure DevOps PR URL or positive PR number. Omit it where allowed to use the current branch.
Work-item commands derive the project from the current Azure Git remote unless --project is supplied.
Command data is written as JSON; open commands print the URL before launching it.";
