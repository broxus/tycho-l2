use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use aide::axum::routing::get_with;
use aide::axum::ApiRouter;
use aide::transform::TransformOperation;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Router};
use everscale_types::boc::Boc;
use proof_api_util::api::{
    cache_for, dont_cache, get_version, prepare_open_api, ApiRouterExt, OpenApiConfig, JSON_HEADERS,
};
use proof_api_util::serde_helpers::TonAddr;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tycho_util::sync::rayon_run;

use crate::storage::ProofStorage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub listen_addr: SocketAddr,
    pub public_url: Option<String>,
}

impl Default for ApiConfig {
    #[inline]
    fn default() -> Self {
        Self {
            listen_addr: (Ipv4Addr::LOCALHOST, 8080).into(),
            public_url: None,
        }
    }
}

pub fn build_api(config: &ApiConfig, proofs: ProofStorage) -> Router {
    // Prepare middleware
    let mut open_api = prepare_open_api(OpenApiConfig {
        name: "proof-api-l2",
        public_url: config.public_url.clone(),
        version: crate::BIN_VERSION,
        build: crate::BIN_BUILD,
    });

    let public_api = ApiRouter::new()
        .api_route("/", get_version(crate::BIN_VERSION, crate::BIN_BUILD))
        .api_route(
            "/v1/proof_chain/:address/:lt",
            get_with(get_proof_chain_v1, get_proof_chain_v1_docs),
        )
        .with_docs()
        .layer(
            ServiceBuilder::new()
                .layer(DefaultBodyLimit::max(32))
                .layer(CorsLayer::permissive())
                .layer(TimeoutLayer::new(Duration::from_secs(1))),
        );

    public_api
        .finish_api(&mut open_api)
        .layer(Extension(Arc::new(open_api)))
        .with_state(proofs)
}

// === V1 Routes ===

/// Block proof chain for an existing transaction.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProofChainResponse {
    /// Base64 encoded BOC with the proof chain.
    pub proof_chain: String,
}

async fn get_proof_chain_v1(
    State(state): State<ProofStorage>,
    Path((TonAddr(address), lt)): Path<(TonAddr, u64)>,
) -> Response {
    match state.build_proof(&address, lt).await {
        Ok(Some(proof_chain)) => {
            rayon_run(move || {
                let data = serde_json::to_vec(&ProofChainResponse {
                    proof_chain: Boc::encode_base64(proof_chain),
                })
                .unwrap();

                cache_for(&JSON_HEADERS, axum::body::Bytes::from(data), 604800).into_response()
            })
            .await
        }
        Ok(None) => res_error(ErrorResponse::NotFound {
            message: "tx not found",
        }),
        Err(e) => res_error(ErrorResponse::Internal {
            message: e.to_string(),
        }),
    }
}

fn get_proof_chain_v1_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.description("Enqueue the participant for the reward claim")
        .tag("proof-api-l2")
        .response::<200, axum::Json<ProofChainResponse>>()
        .response::<404, ()>()
        .response::<500, axum::Json<ErrorResponse>>()
}

/// General error response.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "error")]
pub enum ErrorResponse {
    Internal { message: String },
    NotFound { message: &'static str },
}

fn res_error(error: ErrorResponse) -> Response {
    let status = match &error {
        ErrorResponse::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        ErrorResponse::NotFound { .. } => StatusCode::NOT_FOUND,
    };

    let data = serde_json::to_vec(&error).unwrap();
    (
        status,
        dont_cache(&JSON_HEADERS, axum::body::Bytes::from(data)),
    )
        .into_response()
}
