use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{Context, Result};
use cadastre::config::{heartbeat_file_from_env, Config};
use cadastre::daemon::{self, Timing};
use cadastre::docker::Docker;
use cadastre::heartbeat;
use cadastre::pipeline::run_once;
use cadastre::timefmt::unix_now;
use cadastre::{log, VERSION};

const USAGE: &str = "\
cadastre - keep a folder of Markdown up to date with your Docker containers

Usage:
  cadastre run                    Run as a service: react to Docker events,
                                  reconcile periodically
  cadastre once [--accept-shrink] Document the containers once and exit
  cadastre healthcheck            Exit 0 if a running service is healthy
  cadastre --version
  cadastre --help

Configuration is read from CADASTRE_* environment variables; see
docs/configuration.md.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["--version" | "-V"] => {
            println!("cadastre {VERSION}");
            ExitCode::SUCCESS
        }
        ["--help" | "-h"] => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        ["run"] => finish(run()),
        ["once"] => finish(once(false)),
        ["once", "--accept-shrink"] => finish(once(true)),
        ["healthcheck"] => finish(health()),
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn finish(result: Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error(format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let config = Config::from_env()?;
    let shutdown = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(signal, Arc::clone(&shutdown))
            .context("cannot install the signal handlers")?;
    }
    log::info(format!(
        "cadastre {VERSION} starting: docker={} out={} reconcile={}s debounce={}s",
        config.docker_url,
        config.out_dir.display(),
        config.reconcile.as_secs(),
        config.debounce.as_secs()
    ));
    let docker = Docker::new(&config.docker_url);
    daemon::run(&config, &docker, &Timing::default(), &shutdown)
}

fn once(accept_shrink: bool) -> Result<()> {
    let config = Config::from_env()?;
    let docker = Docker::new(&config.docker_url);
    let outcome = run_once(&config, &docker, unix_now(), accept_shrink)?;
    println!(
        "{} container(s): {} page(s) written, {} removed{}",
        outcome.containers,
        outcome.report.written.len(),
        outcome.report.removed.len(),
        if outcome.changelog_entry {
            ", changelog updated"
        } else {
            ""
        }
    );
    Ok(())
}

fn health() -> Result<()> {
    heartbeat::check(
        &heartbeat_file_from_env(),
        unix_now(),
        heartbeat::MAX_AGE_SECONDS,
    )
}
