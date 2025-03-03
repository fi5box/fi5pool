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

use core::fmt;
use std::collections::HashMap;

use ckb_jsonrpc_types::BlockTemplate;
use ckb_types::{H256, U256, packed::Block, prelude::Unpack, utilities::compact_to_target};
use ringbuf::{
    StaticRb,
    traits::{Consumer, RingBuffer},
};
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct MiningJob {
    pub pow_id: String,
    pub work_id: u64,
    pub height: u64,
    pub timestamp: u64,
    pub target: U256,
    pub parent_hash: H256,
    pub pow_hash: H256,
    #[serde(skip)]
    pub block: Block,
}

impl fmt::Debug for MiningJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MiningJob")
            .field("pow_id", &self.pow_id)
            .field("work_id", &self.work_id)
            .field("height", &self.height)
            .field("timestamp", &self.timestamp)
            .field("target", &hex::encode(self.target.to_be_bytes()))
            .field("parent_hash", &self.parent_hash.to_string())
            .field("pow_hash", &self.pow_hash.to_string())
            .finish()
    }
}

impl From<BlockTemplate> for MiningJob {
    fn from(block_template: BlockTemplate) -> Self {
        let work_id = block_template.work_id.into();
        let height = block_template.number.into();
        let timestamp = block_template.current_time.into();
        let parent_hash = block_template.parent_hash.clone();
        let block: Block = block_template.into();
        let header = block.header();
        let (target, _) = compact_to_target(header.raw().compact_target().unpack());
        let pow_hash = H256(header.as_reader().calc_pow_hash().unpack());
        let pow_id = pow_hash.to_string()[..8].to_string();
        MiningJob {
            pow_id,
            work_id,
            height,
            timestamp,
            target,
            parent_hash,
            pow_hash,
            block,
        }
    }
}

const RB_SIZE: usize = 32;

pub struct Repository {
    last_height: u64,
    jobs: HashMap<String, MiningJob>,
    hashes: StaticRb<String, RB_SIZE>,
}

impl Repository {
    pub fn new() -> Self {
        Repository {
            jobs: HashMap::new(),
            hashes: StaticRb::<String, RB_SIZE>::default(),
            last_height: 0,
        }
    }

    #[inline]
    pub fn on_block_template(&mut self, block_template: BlockTemplate) -> Option<MiningJob> {
        let job: MiningJob = block_template.into();
        if job.height >= self.last_height && self.hashes.last() != Some(&job.pow_id) {
            self.insert_job(&job);
            Some(job)
        } else {
            None
        }
    }

    #[inline]
    pub fn get_job(&self, pow_id: &str) -> Option<&MiningJob> {
        self.jobs.get(pow_id)
    }

    #[inline]
    fn insert_job(&mut self, job: &MiningJob) {
        if let Some(old_id) = self.hashes.push_overwrite(job.pow_id.clone()) {
            self.jobs.remove(&old_id);
        }
        self.last_height = job.height;
        self.jobs.insert(job.pow_id.clone(), job.clone());
    }

    #[inline]
    pub fn last_job(&self) -> Option<MiningJob> {
        self.hashes
            .last()
            .and_then(|hash| self.jobs.get(hash).cloned())
    }

    #[inline]
    pub const fn last_height(&self) -> u64 {
        self.last_height
    }
}
