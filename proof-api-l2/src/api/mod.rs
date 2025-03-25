use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use aide::axum::routing::{get, get_with};
use aide::axum::ApiRouter;
use aide::openapi::OpenApi;
use aide::scalar::Scalar;
use aide::transform::TransformOperation;
use anyhow::Result;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{self, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use everscale_types::boc::Boc;
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tycho_util::sync::rayon_run;

use self::models::{ApiInfoResponse, ErrorResponse, ProofChainResponse, TonAddr};
use crate::storage::ProofStorage;

pub mod models;

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

pub struct Api {
    serve_fn: Box<dyn FnOnce() -> BoxFuture<'static, Result<()>> + Send>,
}

impl Api {
    pub async fn bind(config: ApiConfig, proofs: ProofStorage) -> Result<Self> {
        // Prepare middleware
        let mut open_api = prepare_open_api(&config);

        let public_api = ApiRouter::new()
            .api_route("/", get_with(get_version, get_version_docs))
            .api_route(
                "/v1/proof_chain/:address/:lt",
                get_with(get_proof_chain_v1, get_proof_chain_v1_docs),
            )
            .route("/docs", Scalar::new("api.json").axum_route())
            .api_route_with("/api.json", get(get_api_json), |op| op.tag("swagger"))
            .layer(
                ServiceBuilder::new()
                    .layer(DefaultBodyLimit::max(32))
                    .layer(CorsLayer::permissive())
                    .layer(TimeoutLayer::new(Duration::from_secs(1))),
            );

        let app = public_api
            .finish_api(&mut open_api)
            .layer(Extension(Arc::new(open_api)))
            .with_state(proofs);

        let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
        tracing::info!(listen_addr = %config.listen_addr, "started listening");

        Ok(Self {
            serve_fn: Box::new(move || {
                Box::pin(async move { axum::serve(listener, app).await.map_err(Into::into) })
            }),
        })
    }

    pub async fn serve(self) -> Result<()> {
        (self.serve_fn)().await
    }
}

// === General Routes ===

fn get_version() -> futures_util::future::Ready<Response> {
    static RESPONSE: OnceLock<Vec<u8>> = OnceLock::new();

    let res = RESPONSE.get_or_init(|| {
        simd_json::to_vec(&ApiInfoResponse {
            version: crate::BIN_VERSION.to_owned(),
            build: crate::BIN_BUILD.to_owned(),
        })
        .unwrap()
    });

    let data = axum::body::Bytes::from_static(res);
    futures_util::future::ready((JSON_HEADERS, data).into_response())
}

fn get_version_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.description("Get the API version")
        .response::<200, axum::Json<ApiInfoResponse>>()
}

// === V1 Routes ===

async fn get_proof_chain_v1(
    State(state): State<ProofStorage>,
    Path((TonAddr(address), lt)): Path<(TonAddr, u64)>,
) -> Response {
    match state.build_proof(&address, lt).await {
        Ok(Some(proof_chain)) => {
            rayon_run(move || {
                let data = simd_json::to_vec(&ProofChainResponse {
                    proof_chain: Boc::encode_base64(proof_chain),
                })
                .unwrap();

                (JSON_HEADERS, axum::body::Bytes::from(data)).into_response()
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

// === Doc Routes ===

fn get_api_json(Extension(api): Extension<Arc<OpenApi>>) -> futures_util::future::Ready<Response> {
    static JSON: OnceLock<String> = OnceLock::new();

    let json: &'static [u8] = JSON
        .get_or_init(|| serde_json::to_string(&*api).unwrap())
        .as_bytes();

    let data = axum::body::Bytes::from_static(json);
    futures_util::future::ready((JSON_HEADERS, data).into_response())
}

// === Other stuff ===

fn prepare_open_api(config: &ApiConfig) -> OpenApi {
    use aide::openapi::{Info, Server};

    use crate::{BIN_BUILD, BIN_VERSION};

    let mut servers = vec![Server {
        url: "http://127.0.0.1:8080".to_owned(),
        description: Some("local".to_string()),
        variables: Default::default(),
        extensions: Default::default(),
    }];

    if let Some(public_url) = &config.public_url {
        servers.push(Server {
            url: public_url.clone(),
            description: Some("production".to_string()),
            variables: Default::default(),
            extensions: Default::default(),
        });
    }

    OpenApi {
        info: Info {
            version: format!("{BIN_VERSION} (build {BIN_BUILD})"),
            description: Some("proof-api-l2".to_string()),
            ..Info::default()
        },
        servers,
        ..OpenApi::default()
    }
}

fn res_error(error: ErrorResponse) -> Response {
    let status = match &error {
        ErrorResponse::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        ErrorResponse::NotFound { .. } => StatusCode::NOT_FOUND,
    };

    let data = simd_json::to_vec(&error).unwrap();
    (status, JSON_HEADERS, axum::body::Bytes::from(data)).into_response()
}

const JSON_HEADERS: [(http::HeaderName, HeaderValue); 1] = [(
    http::header::CONTENT_TYPE,
    HeaderValue::from_static("application/json"),
)];
