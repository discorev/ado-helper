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

const HELP: &str = "Usage:\n  ado auth add NAME --org URL [--no-browser]\n  ado auth update NAME [--no-browser]\n  ado auth status [--check]\n  ado auth remove NAME\n  ado pr show [TARGET] [--profile NAME]\n  ado pr threads [TARGET] [--profile NAME]\n  ado pr changes [TARGET] [--profile NAME] [--iteration N]\n  ado pr clone [TARGET] [--profile NAME] [--directory PATH]\n  ado pr diff [TARGET] [--profile NAME] [--directory PATH]\n  ado pr comment [TARGET] --file PATH --line N [--end-line N] --side left|right\n      --body-file PATH --commit SHA --iteration N --change-id N [--profile NAME]\n\nTARGET is an Azure DevOps PR URL or positive PR number. Omit it to use the current branch.\nPull-request data is written as JSON; authentication and help are human-readable.";
