// Copyright 2019-2023 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use crate::cli::{
    cli::{cli_error_and_die, Cli},
    subcommand,
};
use crate::cli_shared::{cli::LogConfig, logger};
use crate::utils::io::ProgressBar;
use clap::Parser;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    // Capture Cli inputs
    let Cli { opts, cmd } = Cli::parse();

    match opts.to_config() {
        Ok((cfg, _)) => {
            logger::setup_logger(&cfg.log, &opts);
            ProgressBar::set_progress_bars_visibility(cfg.client.show_progress_bars);
            if opts.dry_run {
                return Ok(());
            }
            subcommand::process(cmd, cfg, &opts).await
        }
        Err(e) => {
            logger::setup_logger(&LogConfig::default(), &opts);
            cli_error_and_die(format!("Error parsing config: {e}"), 1);
        }
    }
}