// Copyright Fi5 LLC.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#[macro_use]
extern crate tracing as logger;
use openssl as _;

mod ckb_rpc;
mod config;
mod http;
mod influx;
mod job;
mod json_rpc;
mod pool;
mod worker;

use ckb_types::U256;
use clap::Parser;
use color_eyre::Result;
use config::PoolConfig;
use pool::pool_listen;
use tokio::net::TcpListener;

#[derive(Parser, Debug, Clone)]
#[command(author, version)]
pub struct Args {
    #[clap(short('c'), long = "config", default_value = "config.toml")]
    config_path: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let config: PoolConfig = common_x::configure::file_config(&args.config_path)?;
    common_x::log::init_log(config.log_config.clone());
    info!("Config: {:#?}", config);
    let difficulty: U256 = config.difficulty.into();
    let difficulty_for_log = config.difficulty as f64 / 1_000_000f64;
    let target = worker::difficulty_to_target(&difficulty);
    info!(
        "Difficulty: {}, difficulty_for_log: {}, target: {}",
        difficulty,
        difficulty_for_log,
        hex::encode(target.to_be_bytes()),
    );

    let listener = TcpListener::bind(format!("[::]:{}", config.rpc_listen_port)).await?;
    info!("Server listening on: {}", listener.local_addr()?);

    pool_listen(listener, config, args.config_path).await
}
