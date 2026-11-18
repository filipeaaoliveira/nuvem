use std::process::ExitCode;

use clap::Parser;
use nuvem::{Cli, Command, Config, run};

// `ExitCode` rather than `anyhow::Result<()>`: the `Result` return reports
// every failure as 1, so "there is no terminal here" and "the session ended
// badly" reached a script as the same number. See `nuvem::exit` for the table.
//
// Returning `ExitCode` means this function owns the reporting too, which
// `Termination for Result` did for free. The `{error:?}` below is what it
// printed — anyhow's `Debug` renders the message plus its context chain, and
// `Display` would show only the outermost line.
#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Some(Command::Info) = cli.command {
        print!("{}", nuvem::config::run_info(&cli));
        return ExitCode::SUCCESS;
    }

    let outcome = Config::resolve(&cli);
    for warning in outcome.problems.messages() {
        eprintln!("warning: {warning}");
    }

    match run(outcome, cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:?}");
            ExitCode::from(nuvem::exit::code_for(&error))
        }
    }
}
