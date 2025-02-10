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

use crate::{
    ckb_rpc,
    config::{BlockAssembler, PoolConfig},
    http,
    job::Repository,
    json_rpc::{Message, Notification},
    worker::Worker,
};

use ckb_jsonrpc_types::{BlockTemplate, JsonBytes};
use ckb_types::{packed, prelude::*};
use color_eyre::Result;
use common_x::signal::waiting_for_shutdown;
use futures::stream;
use influxdb2::{api::write::TimestampPrecision, models::DataPoint, Client};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use std::{path::Path, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{broadcast, RwLock},
};
use tracing::info;

pub async fn pool_listen(
    listener: TcpListener,
    mut config: PoolConfig,
    config_path: String,
) -> Result<()> {
    let job_repository = Arc::new(RwLock::new(Repository::new()));
    let influx_client = config
        .influx
        .clone()
        .map(|c| Client::new(c.url, c.org, c.token));
    let (broadcast_tx, mut broadcast_rv) = broadcast::channel::<Message>(10);

    tokio::spawn(http::serve(
        config.clone(),
        job_repository.clone(),
        broadcast_tx.clone(),
    ));

    // reload config
    let config_path_clone = config_path.clone();
    let broadcast_tx_ = broadcast_tx.clone();
    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| {
            if let Ok(event) = result {
                if event.kind.is_modify() {
                    match common_x::configure::file_config::<PoolConfig>(&config_path_clone) {
                        Ok(new_config) => {
                            info!("reloading config: {:#?}", new_config);
                            broadcast_tx_
                                .send(Message::Config(Box::new(new_config)))
                                .ok();
                        }
                        Err(error) => error!("Error reloading config: {:?}", error),
                    }
                }
            }
        },
        notify::Config::default(),
    )?;
    watcher.watch(Path::new(&config_path), RecursiveMode::Recursive)?;

    let mut pull_block_template_interval = tokio::time::interval(std::time::Duration::from_millis(
        config.pull_block_template_interval,
    ));

    let mut log_interval =
        tokio::time::interval(std::time::Duration::from_secs(config.log_interval));

    loop {
        tokio::select! {
            // pull block template
            _ = pull_block_template_interval.tick() => {
                if let Ok(block_template) = ckb_rpc::pull_block_template(&config.ckb_rpc_url, config.rpc_timeout).await {
                    handle_block_template(
                        block_template,
                        &job_repository,
                        config.block_assembler.clone(),
                        &broadcast_tx,
                    )
                    .await;
                }
            }
            _ = log_interval.tick() => {
                if let Some(influx) = &influx_client {
                    if let Some(c) = &config.influx {
                        let point = DataPoint::builder("status")
                            .tag("pool", &config.pool_id)
                            .tag("node", &config.ckb_rpc_url)
                            .field("conn_count", broadcast_tx.receiver_count() as i64 -1);
                        let points = vec![point.build()?];
                        influx
                            .write_with_precision(
                                &c.log_bucket,
                                stream::iter(points),
                                TimestampPrecision::Nanoseconds,
                            )
                            .await?;
                    }
                };
            }
            Ok((stream, addr)) = listener.accept() => {
                Worker::run(
                    addr,
                    stream,
                    job_repository.clone(),
                    broadcast_tx.subscribe(),
                    config.clone(),
                    influx_client.clone(),
                )
                .await;
            }
            Ok(notify) = broadcast_rv.recv() => {
                if let Message::Config(new_config) = notify {
                    config = *new_config;
                    pull_block_template_interval.reset();
                    pull_block_template_interval = tokio::time::interval(std::time::Duration::from_millis(
                        config.pull_block_template_interval,
                    ));
                    log_interval.reset();
                    log_interval = tokio::time::interval(std::time::Duration::from_secs(
                        config.log_interval,
                    ));
                }
            }
            _ = waiting_for_shutdown() => {
                break Ok(());
            }
        }
    }
}

#[inline]
pub async fn handle_block_template(
    mut block_template: BlockTemplate,
    job_repository: &Arc<RwLock<Repository>>,
    block_assembler: Option<BlockAssembler>,
    broadcast_tx: &broadcast::Sender<Message>,
) {
    // change coinbase
    if let Some(block_assembler) = block_assembler {
        let origin_witness: packed::CellbaseWitness = packed::CellbaseWitness::from_slice(
            block_template.cellbase.data.witnesses[0].as_bytes(),
        )
        .expect("bound checked");

        // remove prefix length bytes
        let mut message = origin_witness.message().as_bytes().to_vec()[4..].to_vec();

        if !block_assembler.message.is_empty() {
            message.extend_from_slice(b" ");
            message.extend_from_slice(block_assembler.message.as_bytes());
        }

        let script: packed::Script = block_assembler.script.clone().into();
        let cellbase_witness = packed::CellbaseWitness::new_builder()
            .lock(script)
            .message(message.pack())
            .build();

        block_template.cellbase.data.witnesses[0] =
            JsonBytes::from_bytes(cellbase_witness.as_bytes());
        let coinbase: packed::Transaction = block_template.cellbase.data.clone().into();
        block_template.cellbase.hash = coinbase.calc_tx_hash().unpack();
    }

    let job = job_repository
        .write()
        .await
        .on_block_template(block_template);
    if let Some(job) = job {
        debug!("New Job: {:#?}", job);
        let params = json!([
            // job_id
            job.pow_id,
            // header_hash
            job.pow_hash.to_string(),
            // height
            job.height,
            // parent_hash
            job.parent_hash.to_string(),
            // clean_job
            true,
        ]);
        broadcast_tx
            .send(Message::Notification(Notification {
                id: (),
                method: "mining.notify".to_string(),
                params,
            }))
            .ok();
    }
}
