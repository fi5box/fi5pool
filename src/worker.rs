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
    config::PoolConfig,
    job::{MiningJob, Repository},
    json_rpc::{Message, Notification, Request, Response},
};

use ckb_types::{U256, prelude::*};
use color_eyre::{Result, eyre::eyre};
use futures::{
    SinkExt, StreamExt,
    stream::{self, SplitSink},
};
use influxdb2::{Client, api::write::TimestampPrecision, models::DataPoint};
use serde::Serialize;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::TcpStream,
    sync::{RwLock, broadcast::Receiver},
};
use tokio_util::codec::{Framed, LinesCodec};
use tracing::{info, warn};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
enum Stage {
    Init,
    Auth,
    Subscribe,
    SetTarget,
    InitJob,
    Mining,
}

#[inline]
pub fn difficulty_to_target(difficulty: &U256) -> U256 {
    let target = U256::max_value();
    target / difficulty
}

pub struct Worker {
    config: PoolConfig,
    difficulty_for_log: f64,
    target: U256,

    stage: Stage,
    extranonce1: String,
    extranonce2_size: usize,
    job_repository: Arc<RwLock<Repository>>,
    influx_client: Option<Client>,

    miner: Option<String>,
    worker: Option<String>,
    label: String,
    addr: SocketAddr,
    online_timestamp: chrono::DateTime<chrono::Utc>,
}

impl Worker {
    pub fn new(
        addr: SocketAddr,
        config: PoolConfig,
        job_repository: Arc<RwLock<Repository>>,
        influx_client: Option<Client>,
    ) -> Self {
        let extranonce1 = hex::encode(rand::random::<[u8; 4]>());
        let difficulty: U256 = config.difficulty.into();
        Self {
            stage: Stage::Init,
            target: difficulty_to_target(&difficulty),
            difficulty_for_log: config.difficulty as f64 / 1_000_000f64,
            config,
            label: extranonce1.clone(),
            extranonce1,
            extranonce2_size: 12,
            job_repository,
            influx_client,
            miner: None,
            worker: None,
            addr,
            online_timestamp: chrono::Utc::now(),
        }
    }

    pub async fn run(
        addr: SocketAddr,
        stream: TcpStream,
        job_repository: Arc<RwLock<Repository>>,
        broadcast_rv: Receiver<Message>,
        config: PoolConfig,
        influx_client: Option<Client>,
    ) {
        let lines = Framed::new(stream, LinesCodec::new());
        let mut worker = Worker::new(addr, config, job_repository, influx_client);
        info!("[{}] - New worker from: {}", worker.label, worker.addr);
        tokio::spawn(async move {
            if let Err(e) = worker.mining(broadcast_rv, lines).await {
                let msg = format!("[{}] - worker offline: {e}", worker.label);
                warn!("{msg}");
                worker.alert(Alert::Disconnected, msg).await;
            }
        });
    }

    async fn mining(
        &mut self,
        mut broadcast_rv: Receiver<Message>,
        lines: Framed<TcpStream, LinesCodec>,
    ) -> Result<()> {
        let (mut writer, mut reader) = lines.split::<String>();
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let mut timeout_to_mining =
            tokio::time::interval(Duration::from_secs(self.config.timeout_to_mining));
        // not to wait for the first interval
        timeout_to_mining.tick().await;
        loop {
            tokio::select! {
                _ = interval.tick(), if self.stage == Stage::SetTarget || self.stage == Stage::InitJob => match self.stage {
                    Stage::SetTarget => {
                        let req = Request {
                            id: 0,
                            method: "mining.set_target".to_owned(),
                            params: [json!(hex::encode(self.target.to_be_bytes()))].to_vec(),
                        };
                        info!("[{}] - Sending: {:?}", self.label, &req);
                        if writer
                            .send(serde_json::to_string(&Message::Request(req))?)
                            .await
                            .is_ok()
                        {
                            self.stage = Stage::InitJob;
                            info!("[{}] - Stage: SetTarget -> InitJob", self.label);
                        }
                    }
                    Stage::InitJob => {
                        if let Some(job) = self.job_repository.read().await.last_job() {
                            let notify = Notification {
                                id: (),
                                method: "mining.notify".to_string(),
                                params: json!([
                                    // job_id
                                    job.pow_id,
                                    // header_hash
                                    job.pow_hash.to_string(),
                                    // height
                                    job.height,
                                    // parent_hash
                                    job.parent_hash.to_string(),
                                    // clean_job
                                    false,
                                ]),
                            };
                            info!("[{}] - Sending: {:?}", self.label, &notify);
                            if writer
                                .send(serde_json::to_string(&Message::Notification(notify))?)
                                .await
                                .is_ok()
                            {
                                self.stage = Stage::Mining;
                                let msg = format!("[{}] - Stage: InitJob -> Mining", self.label);
                                info!("{msg}");
                                self.save_log("Mining", &msg).await.ok();
                            }
                        }
                    }
                    _ => {}
                },
                _ = timeout_to_mining.tick(), if self.stage != Stage::Mining => {
                    break Err(eyre!("time out to mining"));
                },
                msg = reader.next() => match msg {
                    Some(Ok(message)) => {
                        self.parse_message(&mut writer, message).await?;
                    }
                    Some(Err(e)) => return Err(eyre!("Error reading stream: {e}")),
                    None => return Err(eyre!("Error reading stream: none")),
                },
                Ok(notify) = broadcast_rv.recv() => {
                    match notify {
                        Message::Config(new_config) => {
                            self.config = *new_config;
                            let difficulty: U256 = self.config.difficulty.into();
                            self.difficulty_for_log = self.config.difficulty as f64 / 1_000_000f64;
                            self.target = difficulty_to_target(&difficulty);
                            let target = hex::encode(self.target.to_be_bytes());
                            info!(
                                "[{}] - New difficulty: {}, difficulty_for_log: {}, target: {}",
                                self.label, difficulty, self.difficulty_for_log, target
                            );

                            let req = Request {
                                id: 0,
                                method: "mining.set_target".to_owned(),
                                params: [json!(target)].to_vec(),
                            };
                            info!("[{}] - Sending: {:?}", self.label, &req);
                            writer
                                .send(serde_json::to_string(&Message::Request(req))?)
                                .await?;
                            self.stage = Stage::InitJob;
                            info!("[{}] - Stage: SetTarget -> InitJob", self.label);
                        }
                        Message::Notification(_) =>
                            if self.stage == Stage::Mining {
                                debug!("[{}] - Sending: {:?}", self.label, &notify);
                                writer.send(serde_json::to_string(&notify)?).await?;
                            }
                        _ => {}
                    }
                },
            }
        }
    }

    async fn parse_message(
        &mut self,
        writer: &mut SplitSink<Framed<TcpStream, LinesCodec>, String>,
        incoming_message: String,
    ) -> Result<()> {
        debug!("[{}] - Received: {}", self.label, incoming_message);
        if let Ok(message) = serde_json::from_str(&incoming_message) {
            match self.handle_message(message).await {
                Ok(Some(response)) => {
                    debug!("[{}] - Sending: {:?}", self.label, &response);
                    writer
                        .send(serde_json::to_string(&Message::Response(response))?)
                        .await?
                }
                Err(e) => warn!("[{}] - Error handling message: {}", self.label, e),
                _ => (),
            };
        }
        Ok(())
    }
}

impl Worker {
    async fn handle_message(&mut self, msg: Message) -> Result<Option<Response>>
    where
        Self: std::marker::Sized,
    {
        match msg {
            Message::Request(req) => self.handle_request(req).await,
            _ => Err(eyre!("Invalid request: {:?}", msg)),
        }
    }

    async fn handle_request(&mut self, request: Request) -> Result<Option<Response>>
    where
        Self: std::marker::Sized,
    {
        match request.method.as_str() {
            "mining.authorize" => {
                let authorized = self.handle_authorize(&request.params);
                match self.stage {
                    Stage::Init => {
                        self.stage = Stage::Auth;
                        info!("[{}] - Stage: Init -> Auth", self.label);
                    }
                    Stage::Subscribe => {
                        self.stage = Stage::SetTarget;
                        info!("[{}] - Stage: Subscribe -> SetTarget", self.label);
                    }
                    _ => (),
                }
                Ok(Some(Response::ok(request.id, authorized)))
            }
            "mining.subscribe" => {
                let subscriptions = self.handle_subscribe(&request.params);
                let extra_n1 = self.set_extranonce1(None);
                let extra_n2_size = self.set_extranonce2_size(None);

                match self.stage {
                    Stage::Init => {
                        self.stage = Stage::Subscribe;
                        info!("[{}] - Stage: Init -> Subscribe", self.label);
                    }
                    Stage::Auth => {
                        self.stage = Stage::SetTarget;
                        info!("[{}] - Stage: Auth -> SetTarget", self.label);
                    }
                    _ => (),
                }

                Ok(Some(Response::ok(
                    request.id,
                    json!([subscriptions, extra_n1, extra_n2_size,]),
                )))
            }
            "mining.submit" => {
                // TODO username
                let _user_name = request.params[0]
                    .as_str()
                    .ok_or(eyre!("Invalid Submission: no username"))?;

                let pow_id = request.params[1]
                    .as_str()
                    .ok_or(eyre!("Invalid Submission: no pow_hash"))?;
                let nonce2 = request.params[2]
                    .as_str()
                    .ok_or(eyre!("Invalid Submission: no nonce2"))?;

                // invalid type
                if self.extranonce2_size() * 2 != nonce2.len() {
                    self.save_shares(ShareType::Invalid).await?;
                    warn!("[{}] - Invalid Submission: invalid nonce2 size", self.label);
                    return Ok(Some(Response::ok(request.id, false)));
                }

                let (last_height, job) = {
                    let repository = self.job_repository.read().await;
                    (
                        repository.last_height(),
                        repository.get_job(pow_id).cloned(),
                    )
                };

                if let Some(job) = job {
                    if job.height < last_height {
                        self.save_shares(ShareType::Stale).await?;
                        warn!(
                            "[{}] - Stale Submission: height({}) < last_height({})",
                            self.label, job.height, last_height
                        );
                        return Ok(Some(Response::ok(request.id, false)));
                    }

                    let nonce_hex = format!("{}{}", self.extranonce1, nonce2);
                    let mut nonce_bytes = hex::decode(&nonce_hex)?;
                    nonce_bytes.reverse();
                    let nonce = u128::from_be_bytes(nonce_bytes.try_into().expect("bound checked"));

                    let input = pow_message(&job.pow_hash.to_string(), nonce);
                    let mut output = [0u8; 32];
                    eaglesong::eaglesong(&input, &mut output);

                    let output = U256::from_big_endian(&output).expect("bound checked");

                    info!(
                        "[{}] - Submit nonce: {}, pow_hash: {}, output: {}, job_target: {}, pool_target: {}",
                        self.label,
                        nonce_hex,
                        job.pow_hash,
                        hex::encode(output.to_be_bytes()),
                        hex::encode(job.target.to_be_bytes()),
                        hex::encode(self.target.to_be_bytes()),
                    );

                    if output <= job.target {
                        let height = job.height;
                        let result = self.handle_submit(job, nonce).await;

                        debug!("[{}] - Submit({}) result: {result}", self.label, nonce_hex);
                        self.save_shares(ShareType::Valid).await?;
                        self.save_block(height).await?;
                        if !result {
                            self.alert(
                                Alert::Unexpected,
                                format!("[{}] - submit failed: height({})", self.label, height),
                            )
                            .await;
                        }

                        Ok(Some(Response::ok(request.id, true)))
                    } else if output <= self.target {
                        self.save_shares(ShareType::Valid).await?;
                        Ok(Some(Response::ok(request.id, true)))
                    } else {
                        self.save_shares(ShareType::Invalid).await?;
                        warn!(
                            "[{}] - Invalid Submission: Output({}) > Target({})",
                            self.label,
                            hex::encode(output.to_be_bytes()),
                            hex::encode(self.target.to_be_bytes()),
                        );
                        Ok(Some(Response::ok(request.id, false)))
                    }
                } else {
                    self.save_shares(ShareType::Stale).await?;
                    warn!("[{}] - Stale Submission: No job found", self.label);
                    Ok(Some(Response::ok(request.id, false)))
                }
            }
            _ => Ok(None),
        }
    }

    const fn handle_subscribe(&self, _request: &[Value]) -> Vec<(String, String)> {
        vec![]
    }

    fn handle_authorize(&mut self, params: &[Value]) -> bool {
        if params.is_empty() {
            return false;
        }

        if let Some(auth_code) = params[0].as_str() {
            if let Some((miner_name, worker_name)) = auth_code.split_once('.') {
                self.miner = Some(miner_name.to_string());
                self.worker = Some(worker_name.to_string());
                self.label = format!(
                    "{}-{}.{}",
                    self.extranonce1,
                    if miner_name.len() > 8 {
                        miner_name.split_at(miner_name.len() - 8).1
                    } else {
                        miner_name
                    },
                    if worker_name.len() > 8 {
                        worker_name.split_at(worker_name.len() - 8).1
                    } else {
                        worker_name
                    },
                );
                return true;
            }
        }
        false
    }

    async fn handle_submit(&self, job: MiningJob, nonce: u128) -> bool {
        let header = job.block.header().as_builder().nonce(nonce.pack()).build();

        let block = job
            .block
            .clone()
            .as_advanced_builder()
            .header(header.into_view())
            .build();

        ckb_rpc::submit_block(
            &self.config.ckb_rpc_url,
            &job.work_id.to_string(),
            block.data().into(),
            self.config.rpc_timeout,
            self.config.rpc_retry_times,
        )
        .await
        .map_err(|e| {
            warn!("[{}] - ckb_rpc submit_block failed: {}", self.label, e);
            eyre!(e)
        })
        .is_ok()
    }

    fn set_extranonce1(&mut self, extranonce1: Option<String>) -> String {
        if let Some(extranonce1) = extranonce1 {
            self.extranonce1 = extranonce1;
        }
        self.extranonce1.clone()
    }

    fn set_extranonce2_size(&mut self, extra_nonce2_size: Option<usize>) -> usize {
        if let Some(extra_nonce2_size) = extra_nonce2_size {
            self.extranonce2_size = extra_nonce2_size;
        }
        self.extranonce2_size
    }

    const fn extranonce2_size(&self) -> usize {
        self.extranonce2_size
    }
}

#[derive(Debug)]
enum ShareType {
    Valid,
    Stale,
    Invalid,
}

// influxdb
impl Worker {
    async fn save_shares(&self, share_type: ShareType) -> Result<()> {
        if let Some(influx) = &self.influx_client {
            if let Some(c) = &self.config.influx {
                let point = DataPoint::builder("worker")
                    .tag("miner", self.miner.clone().unwrap_or_default())
                    .tag("worker", self.worker.clone().unwrap_or_default());
                let point = match share_type {
                    ShareType::Valid => point.field("valid_shares", self.difficulty_for_log),
                    ShareType::Stale => point.field("stale_shares", 1),
                    ShareType::Invalid => point.field("invalid_shares", 1),
                };

                let points = vec![point.build()?];

                influx
                    .write_with_precision(
                        &c.share_bucket,
                        stream::iter(points),
                        TimestampPrecision::Nanoseconds,
                    )
                    .await?;
            }
        };
        Ok(())
    }

    async fn save_block(&self, height: u64) -> Result<()> {
        if let Some(influx) = &self.influx_client {
            if let Some(c) = &self.config.influx {
                let point = DataPoint::builder("block")
                    .tag("miner", self.miner.clone().unwrap_or_default())
                    .tag("node", self.config.ckb_rpc_url.clone())
                    .field("height", height as i64);

                let points = vec![point.build()?];

                influx
                    .write_with_precision(
                        &c.block_bucket,
                        stream::iter(points),
                        TimestampPrecision::Nanoseconds,
                    )
                    .await?;
            }
        };
        Ok(())
    }

    async fn save_log(&self, field: &str, value: &str) -> Result<()> {
        if let Some(influx) = &self.influx_client {
            if let Some(c) = &self.config.influx {
                let point = DataPoint::builder("log")
                    .tag("worker", self.worker.clone().unwrap_or_default())
                    .tag("miner", self.miner.clone().unwrap_or_default())
                    .tag("node", self.config.ckb_rpc_url.clone())
                    .field(field, value);

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
        Ok(())
    }
}

#[derive(Debug)]
enum Alert {
    Disconnected,
    Unexpected,
}

impl Worker {
    async fn alert(&self, alert_type: Alert, msg: String) {
        let alert = json!({
            "msg": msg,
            "status": {
                "addr": self.addr,
                "label": self.label,
                "stage": self.stage,
                "extranonce1": self.extranonce1,
                "extranonce2_size":  self.extranonce2_size,
                "miner": self.miner,
                "worker": self.worker,
                "connect_timestamp": self.online_timestamp.to_rfc3339(),
                "offline_timestamp": chrono::Utc::now().to_rfc3339(),
            }
        });

        // influxdb alert
        self.save_log(&format!("{alert_type:?}"), &alert.to_string())
            .await
            .ok();

        // mail alert
        if let Some(mailer_config) = &self.config.mailer {
            if let Ok(mailer) = common_x::mailer::Mailer::new(mailer_config) {
                mailer
                    .send("Fi5Pool Alert", &alert.to_string(), &mailer_config.username)
                    .await;
            }
        }

        // webhook alert
        let alert = json!({
            "msg_type":"interactive",
            "card": {
                "schema": "2.0",
                "config": {
                    "update_multi": true,
                    "style": {
                        "text_size": {
                            "normal_v2": {
                                "default": "normal",
                                "pc": "normal",
                                "mobile": "heading"
                            }
                        }
                    }
                },
                "body": {
                    "direction": "vertical",
                    "padding": "12px 12px 12px 12px",
                    "elements": [
                        {
                            "tag": "markdown",
                            "content": format!("```json\n{}\n```",
                                serde_json::to_string_pretty(&alert).unwrap_or(alert.to_string())
                            ),
                            "text_align": "left",
                            "text_size": "normal_v2",
                            "margin": "0px 0px 0px 0px"
                        }
                    ]
                },
                "header": {
                    "title": {
                        "tag": "plain_text",
                        "content": "Fi5Pool 告警"
                    },
                    "template": "yellow",
                    "icon": {
                        "tag": "standard_icon",
                        "token": "announce_filled"
                    },
                    "padding": "12px 12px 12px 12px"
                }
            }
        });
        if let Some(alert_url) = &self.config.alert_url {
            reqwest::Client::new()
                .post(alert_url)
                .json(&alert)
                .timeout(Duration::from_millis(self.config.rpc_timeout))
                .send()
                .await
                .ok();
        }
    }
}

fn pow_message(pow_hash: &str, nonce: u128) -> [u8; 48] {
    use byteorder::{ByteOrder, LittleEndian};

    let mut message = [0; 48];
    message[0..32].copy_from_slice(&hex::decode(pow_hash).expect("bound checked"));
    LittleEndian::write_u128(&mut message[32..48], nonce);
    message
}

#[test]
fn test_output() -> Result<()> {
    let mut nonce_bytes = hex::decode("47d7babb03000000db7b2d4fa0934f1b")?;
    nonce_bytes.reverse();
    let nonce = u128::from_be_bytes(nonce_bytes.try_into().unwrap());
    let input = pow_message(
        "b8cf1221587b1be055d3ffd4382a901f58a5b7fb9e5fc07da112fc4fe552df0e",
        nonce,
    );
    let mut output = [0u8; 32];
    eaglesong::eaglesong(&input, &mut output);

    let output = U256::from_big_endian(&output).expect("bound checked");

    println!("output: {:?}", &hex::encode(output.to_be_bytes()));
    Ok(())
}
