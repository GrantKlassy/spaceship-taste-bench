use anyhow::{Context, Result};
use bench::{
    archive,
    config::{Cli, Commands, Config, state_dir},
    process, protocol, replay,
    sandbox::Sbx,
    workflow,
};
use clap::Parser;

fn main() {
    if let Err(error) = execute() {
        // Error strings are controller-generated, never raw agent/backend output.
        let safe: String = format!("{error:#}")
            .chars()
            .map(|c| {
                if c.is_control() && c != '\n' {
                    '�'
                } else {
                    c
                }
            })
            .collect();
        eprintln!("bench: {safe}");
        std::process::exit(1);
    }
}
fn execute() -> Result<()> {
    let cli = Cli::parse();
    let repo = cli
        .repo
        .canonicalize()
        .context("repository path unavailable")?;
    archive::ensure_directory(&repo)?;
    let default_config = repo.join("bench.local.toml");
    let config_path = cli
        .config
        .as_deref()
        .or_else(|| default_config.exists().then_some(default_config.as_path()));
    let config = Config::load(config_path)?;
    let backend = Sbx::new(&repo, &config)?;
    match cli.command {
        Commands::Doctor { agent, json } => {
            let report = backend.doctor(agent);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                for line in report.lines() {
                    println!("{line}");
                }
            }
            anyhow::ensure!(
                report.ready,
                "not ready for official attempts; resolve the blocked checks above"
            );
            Ok(())
        }
        Commands::Auth { agent } => backend.auth(agent),
        Commands::Run { agent, model, task } => {
            let state = state_dir(&repo)?;
            let abort = process::signals()?;
            let backend = backend.with_abort(abort.clone());
            workflow::run(
                &backend,
                &config,
                &repo,
                &state,
                workflow::Attempt {
                    agent,
                    model: &model,
                    task_version: &task,
                },
                &abort,
            )?;
            Ok(())
        }
        Commands::Play { run_id } => {
            let (dir, run) = protocol::load_run(&repo, &run_id)?;
            let state = state_dir(&repo)?;
            let abort = process::signals()?;
            let backend = backend.with_abort(abort.clone());
            replay::play(&backend, &config, &dir, &run, &state, &abort)
        }
        Commands::CheckIntegration => {
            let state = state_dir(&repo)?;
            let abort = process::signals()?;
            let backend = backend.with_abort(abort.clone());
            bench::sandbox::integration_check_abort(&backend, &repo, &state, &abort)
        }
        Commands::CheckRuntimes { agent } => {
            let abort = process::signals()?;
            let backend = backend.with_abort(abort);
            let report = backend.check_runtimes(agent)?;
            for line in report.lines() {
                println!("{line}");
            }
            anyhow::ensure!(
                report.ready,
                "agent runtimes still fail required isolation/configuration checks"
            );
            Ok(())
        }
    }
}
