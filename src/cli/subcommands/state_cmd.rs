// Copyright 2019-2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::path::PathBuf;
use std::time::Duration;

use crate::rpc::RpcMethodExt as _;
use crate::shim::clock::ChainEpoch;
use crate::shim::econ::TokenAmount;
use crate::{rpc::state::StateFetchRoot, rpc_client::ApiInfo};
use cid::Cid;
use clap::Subcommand;
use serde_tuple::{self, Deserialize_tuple, Serialize_tuple};

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug)]
struct VestingSchedule {
    entries: Vec<VestingScheduleEntry>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug)]
struct VestingScheduleEntry {
    epoch: ChainEpoch,
    amount: TokenAmount,
}

#[derive(Debug, Subcommand)]
pub enum StateCommands {
    Fetch {
        root: Cid,
        /// The `.car` file path to save the state root
        #[arg(short, long)]
        save_to_file: Option<PathBuf>,
    },
}

impl StateCommands {
    pub async fn run(self, api: ApiInfo) -> anyhow::Result<()> {
        match self {
            Self::Fetch { root, save_to_file } => {
                let ret = api
                    .call(
                        StateFetchRoot::request((root, save_to_file))?.with_timeout(Duration::MAX),
                    )
                    .await?;
                println!("{ret}");
            }
        }
        Ok(())
    }
}
