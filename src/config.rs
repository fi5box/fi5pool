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

use ckb_jsonrpc_types::{JsonBytes, Script};
use common_x::{log::LogConfig, mailer::MailerConfig};
use serde::{Deserialize, Serialize};

use crate::influx::InfluxConfig;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PoolConfig {
    pub rpc_listen_port: u16,
    pub http_listen_port: u16,
    pub pool_id: String,
    pub block_assembler: Option<BlockAssembler>,
    pub ckb_rpc_url: String,
    pub influx: Option<InfluxConfig>,
    pub alert_url: Option<String>,
    pub pull_block_template_interval: u64,
    pub log_interval: u64,
    pub rpc_timeout: u64,
    pub timeout_to_mining: u64,
    pub rpc_retry_times: u8,
    pub difficulty: u64,
    pub log_config: LogConfig,
    pub mailer: Option<MailerConfig>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            rpc_listen_port: 34255,
            http_listen_port: 8888,
            pool_id: hex::encode(rand::random::<[u8; 4]>()),
            block_assembler: None,
            ckb_rpc_url: "http://127.0.0.1:8114".to_string(),
            influx: None,
            alert_url: None,
            pull_block_template_interval: 100,
            log_interval: 120,
            rpc_timeout: 1000,
            timeout_to_mining: 60,
            difficulty: u64::MAX,
            log_config: LogConfig::default(),
            mailer: None,
            rpc_retry_times: 5,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct BlockAssembler {
    #[serde(flatten)]
    pub script: Script,
    pub message: JsonBytes,
}
