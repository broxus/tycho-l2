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
use everscale_types::cell::HashBytes;
use proof_api_util::api::{
    get_version, prepare_open_api, ApiRouterExt, OpenApiConfig, JSON_HEADERS,
};
use proof_api_util::serde_helpers::TonAddr;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tycho_util::sync::rayon_run;

use crate::client::TonClient;

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

pub fn build_api(config: &ApiConfig, client: TonClient) -> Router {
    // Prepare middleware
    let mut open_api = prepare_open_api(OpenApiConfig {
        name: "proof-api-ton",
        public_url: config.public_url.clone(),
        version: crate::BIN_VERSION,
        build: crate::BIN_BUILD,
    });

    let public_api = ApiRouter::new()
        .api_route("/", get_version(crate::BIN_VERSION, crate::BIN_BUILD))
        .api_route(
            "/v1/proof_chain/:address/:lt/:hash",
            get_with(get_proof_chain_v1, get_proof_chain_v1_docs),
        )
        .with_docs()
        .layer(
            ServiceBuilder::new()
                .layer(DefaultBodyLimit::max(32))
                .layer(CorsLayer::permissive())
                .layer(TimeoutLayer::new(Duration::from_secs(10))),
        );

    public_api
        .finish_api(&mut open_api)
        .layer(Extension(Arc::new(open_api)))
        .with_state(client)
}

// === V1 Routes ===

/// Block proof chain for an existing transaction.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProofChainResponse {
    /// Base64 encoded BOC with the proof chain.
    pub proof_chain: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct TxHash(pub HashBytes);

impl schemars::JsonSchema for TxHash {
    fn schema_name() -> String {
        "Transaction hash".to_string()
    }

    fn json_schema(gen: &mut schemars::SchemaGenerator) -> schemars::schema::Schema {
        let schema = gen.subschema_for::<String>();
        let mut schema = schema.into_object();
        schema.metadata().description = Some("Transaction hash as hex".to_string());
        schema.format = Some("[0-9a-fA-F]{64}".to_string());
        schema.metadata().examples = vec![serde_json::json!(
            "3333333333333333333333333333333333333333333333333333333333333333"
        )];
        schema.into()
    }
}

async fn get_proof_chain_v1(
    State(client): State<TonClient>,
    Path((TonAddr(address), lt, TxHash(tx_hash))): Path<(TonAddr, u64, TxHash)>,
) -> Response {
    match client.build_proof(&address, lt, &tx_hash).await {
        Ok(proof_chain) => {
            rayon_run(move || {
                let data = serde_json::to_vec(&ProofChainResponse {
                    proof_chain: Boc::encode_base64(proof_chain),
                })
                .unwrap();

                (JSON_HEADERS, axum::body::Bytes::from(data)).into_response()
            })
            .await
        }
        Err(e) => res_error(ErrorResponse::Internal {
            message: e.to_string(),
        }),
    }
}

fn get_proof_chain_v1_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.description("Enqueue the participant for the reward claim")
        .tag("proof-api-ton")
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
    (status, JSON_HEADERS, axum::body::Bytes::from(data)).into_response()
}
