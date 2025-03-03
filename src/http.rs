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

use std::{sync::Arc, time::Duration};

use ckb_jsonrpc_types::BlockTemplate;
use color_eyre::Result;
use common_x::restful::{
    RESTfulError,
    axum::{
        self, Json, Router,
        extract::State,
        response::IntoResponse,
        routing::{get, post},
    },
    ok_simple,
};
use reqwest::{Method, StatusCode};
use serde_json::json;
use tokio::{
    net::TcpListener,
    sync::{RwLock, broadcast},
};
use tower_http::{cors::CorsLayer, timeout::TimeoutLayer, trace::TraceLayer};

use crate::{
    config::{BlockAssembler, PoolConfig},
    job::Repository,
    json_rpc::Message,
    pool::handle_block_template,
};

#[derive(Clone)]
struct AppState {
    job_repository: Arc<RwLock<Repository>>,
    broadcast_tx: broadcast::Sender<Message>,
    block_assembler: Option<BlockAssembler>,
}

pub async fn serve(
    config: PoolConfig,
    job_repository: Arc<RwLock<Repository>>,
    broadcast_tx: broadcast::Sender<Message>,
) -> Result<()> {
    let router = Router::new()
        .route("/hook/block_template", post(block_template))
        .layer(CorsLayer::new().allow_methods([
            Method::GET,
            Method::POST,
            Method::DELETE,
            Method::PUT,
        ]))
        .layer((
            TraceLayer::new_for_http(),
            TimeoutLayer::new(Duration::from_millis(config.rpc_timeout)),
        ))
        // state
        .with_state(AppState {
            job_repository,
            broadcast_tx,
            block_assembler: config.block_assembler,
        });
    let app = router.route("/health", get(health)).fallback(|| async {
        (
            StatusCode::NOT_FOUND,
            json!({ "code": 404, "message": "Not Found" }).to_string(),
        )
            .into_response()
    });

    let listener = TcpListener::bind(format!("[::]:{}", config.http_listen_port)).await?;
    info!("http listening on [::]:{}", config.http_listen_port);

    Ok(axum::serve(listener, app).await?)
}

async fn health() -> Result<impl IntoResponse, RESTfulError> {
    ok_simple()
}

async fn block_template(
    State(state): State<AppState>,
    Json(block_template): Json<BlockTemplate>,
) -> Result<impl IntoResponse, RESTfulError> {
    debug!("webhook block_template");
    handle_block_template(
        block_template,
        &state.job_repository,
        state.block_assembler,
        &state.broadcast_tx,
    )
    .await;
    ok_simple()
}
