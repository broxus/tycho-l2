use std::convert::Infallible;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use aide::axum::routing::{get, get_with, ApiMethodRouter};
use aide::axum::ApiRouter;
use aide::openapi::OpenApi;
use aide::scalar::Scalar;
use aide::transform::TransformOperation;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use axum::serve::IncomingStream;
use axum::Extension;
use futures_util::future::BoxFuture;
use http::{HeaderName, HeaderValue};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tower_service::Service;

pub struct Api {
    serve_fn: Box<dyn FnOnce() -> BoxFuture<'static, std::io::Result<()>> + Send>,
}

impl Api {
    pub async fn bind<A, M, S>(listen_addr: A, app: M) -> std::io::Result<Self>
    where
        A: Into<SocketAddr>,
        M: for<'a> Service<IncomingStream<'a>, Error = Infallible, Response = S> + Send + 'static,
        S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + 'static,
        for<'a> <M as Service<IncomingStream<'a>>>::Future: Send,
        S::Future: Send,
    {
        let listen_addr = listen_addr.into();
        let listener = tokio::net::TcpListener::bind(listen_addr).await?;
        tracing::info!(%listen_addr, "started api");

        let serve = axum::serve(listener, app);

        Ok(Self {
            serve_fn: Box::new(move || Box::pin(serve.into_future())),
        })
    }

    pub async fn serve(self) -> std::io::Result<()> {
        (self.serve_fn)().await
    }
}

pub trait ApiRouterExt<S> {
    fn with_docs(self) -> Self;
}

impl<S: Clone + Send + Sync + 'static> ApiRouterExt<S> for ApiRouter<S> {
    fn with_docs(self) -> Self {
        self.route("/docs", Scalar::new("api.json").axum_route())
            .api_route_with("/api.json", get(get_api_json), |op| op.tag("swagger"))
    }
}

fn get_api_json(Extension(api): Extension<Arc<OpenApi>>) -> futures_util::future::Ready<Response> {
    static JSON: OnceLock<String> = OnceLock::new();

    let json: &'static [u8] = JSON
        .get_or_init(|| serde_json::to_string(&*api).unwrap())
        .as_bytes();

    let data = axum::body::Bytes::from_static(json);
    futures_util::future::ready((JSON_HEADERS, data).into_response())
}

pub struct OpenApiConfig {
    pub name: &'static str,
    pub public_url: Option<String>,
    pub version: &'static str,
    pub build: &'static str,
}

pub fn prepare_open_api(config: OpenApiConfig) -> OpenApi {
    use aide::openapi::{Info, Server};

    let mut servers = Vec::new();

    if let Some(public_url) = &config.public_url {
        servers.push(Server {
            url: public_url.clone(),
            description: Some("production".to_string()),
            variables: Default::default(),
            extensions: Default::default(),
        });
    }

    servers.push(Server {
        url: "http://127.0.0.1:8080".to_owned(),
        description: Some("local".to_string()),
        variables: Default::default(),
        extensions: Default::default(),
    });

    OpenApi {
        info: Info {
            version: format!("{} (build {})", config.version, config.build),
            description: Some(config.name.to_owned()),
            ..Info::default()
        },
        servers,
        ..OpenApi::default()
    }
}

/// API version and build information.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiInfoResponse {
    pub version: String,
    pub build: String,
}

pub fn get_version<S>(version: &'static str, build: &'static str) -> ApiMethodRouter<S, Infallible>
where
    S: Clone + Send + Sync + 'static,
{
    get_with(move || get_version_impl(version, build), get_version_docs)
}

fn get_version_impl(
    version: &'static str,
    build: &'static str,
) -> futures_util::future::Ready<Response> {
    static RESPONSE: OnceLock<Vec<u8>> = OnceLock::new();

    let res = RESPONSE.get_or_init(|| {
        serde_json::to_vec(&ApiInfoResponse {
            version: version.to_owned(),
            build: build.to_owned(),
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

pub const JSON_HEADERS: [(HeaderName, HeaderValue); 1] = [(
    http::header::CONTENT_TYPE,
    HeaderValue::from_static("application/json"),
)];
