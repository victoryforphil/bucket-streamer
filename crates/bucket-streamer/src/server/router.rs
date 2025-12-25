use std::sync::Arc;

use axum::{http::StatusCode, response::IntoResponse, routing::get, Router};
use tower_http::trace::TraceLayer;

use crate::config::Config;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    // Storage and pipeline components added in Task 11
}

/// Create the Axum router with all routes
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/ws", get(super::websocket::ws_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_check() {
        let state = AppState {
            config: Arc::new(Config::default()),
        };
        let app = create_router(state);

        let response = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
