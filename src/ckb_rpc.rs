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

use std::time::Duration;

use ckb_jsonrpc_types::{Block, BlockTemplate};
use ckb_types::H256;
use color_eyre::eyre::{eyre, Result};
use serde_json::{json, Value};

#[inline]
pub async fn pull_block_template(ckb_jsonrpc_url: &str, time_out: u64) -> Result<BlockTemplate> {
    reqwest::Client::new()
        .post(ckb_jsonrpc_url)
        .json(&json!({"id": 1, "jsonrpc": "2.0", "method": "get_block_template", "params": []}))
        .timeout(Duration::from_millis(time_out))
        .send()
        .await?
        .json::<Value>()
        .await?
        .get("result")
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| eyre!(e)))
        .ok_or(eyre!("jsonrpc get_block_template failed"))?
}

#[inline]
pub async fn submit_block(
    ckb_jsonrpc_url: &str,
    work_id: &str,
    block: Block,
    time_out: u64,
    retry_times: u8,
) -> Result<H256> {
    let result = reqwest::Client::new()
        .post(ckb_jsonrpc_url)
        .json(&json!({"id": 1, "jsonrpc": "2.0", "method": "submit_block", "params": [work_id, block]}))
        .timeout(Duration::from_millis(time_out))
        .send().await;
    if let Ok(result) = result {
        result
            .json::<Value>()
            .await?
            .get("result")
            .map(|v| serde_json::from_value(v.clone()).map_err(|e| eyre!(e)))
            .ok_or(eyre!("jsonrpc submit_block deserialize failed"))?
    } else {
        if retry_times == 0 {
            return Err(eyre!("jsonrpc submit_block failed"));
        }
        Box::pin(submit_block(
            ckb_jsonrpc_url,
            work_id,
            block,
            time_out,
            retry_times - 1,
        ))
        .await
    }
}
