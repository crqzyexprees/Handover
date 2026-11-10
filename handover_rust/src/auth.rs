//! Access control for the local backend.
//!
//! The backend listens on 127.0.0.1, but any web page open in the user's
//! browser can reach localhost. Without protection such a page could list
//! projects and type commands into terminals. Two layers stop that:
//!
//! 1. A per-launch secret token, required on every route except the bare
//!    health check `GET /`: as the `x-handover-token` header for HTTP, or the
//!    `token` query parameter for WebSockets (browsers can't set WebSocket
//!    headers). The desktop shell generates it and hands it to the frontend.
//! 2. CORS limited to the app's own origins, so other pages can't even read
//!    responses.

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tower_http::cors::{AllowOrigin, CorsLayer};

pub const TOKEN_HEADER: &str = "x-handover-token";

/// Origins the frontend is served from: the Tauri webview (Linux/macOS use
/// `tauri://localhost`, Windows `http(s)://tauri.localhost`) and the Vite
/// dev server.
const ALLOWED_ORIGINS: &[&str] = &[
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
    "http://127.0.0.1:5173",
    "http://localhost:5173",
];

#[derive(Clone)]
pub struct AuthToken(pub std::sync::Arc<str>);

impl AuthToken {
    /// A fresh random token (two v4 UUIDs: 244 random bits).
    pub fn generate() -> Self {
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        Self(token.into())
    }

    fn matches(&self, candidate: &str) -> bool {
        constant_time_eq(self.0.as_bytes(), candidate.as_bytes())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn token_from_query(query: Option<&str>) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token").then(|| value.to_string())
    })
}

/// Rejects requests without the right token (except the `GET /` health check
/// the desktop shell polls, and CORS preflights, which carry no headers).
pub async fn require_token(State(token): State<AuthToken>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path == "/" || req.method() == Method::OPTIONS {
        return next.run(req).await;
    }
    let supplied = req
        .headers()
        .get(TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| token_from_query(req.uri().query()));
    match supplied {
        Some(candidate) if token.matches(&candidate) => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "missing or invalid token").into_response(),
    }
}

pub fn cors_layer() -> CorsLayer {
    let origins: Vec<HeaderValue> = ALLOWED_ORIGINS
        .iter()
        .map(|o| HeaderValue::from_static(o))
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            header::CONTENT_TYPE,
            header::HeaderName::from_static(TOKEN_HEADER),
        ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_matching_is_exact() {
        let t = AuthToken("abc123".into());
        assert!(t.matches("abc123"));
        assert!(!t.matches("abc124"));
        assert!(!t.matches("abc12"));
        assert!(!t.matches(""));
    }

    #[test]
    fn token_read_from_query() {
        assert_eq!(
            token_from_query(Some("session=3&token=xyz&since=9")),
            Some("xyz".into())
        );
        assert_eq!(token_from_query(Some("session=3")), None);
        assert_eq!(token_from_query(None), None);
    }

    #[test]
    fn generated_tokens_are_long_and_unique() {
        let a = AuthToken::generate();
        let b = AuthToken::generate();
        assert_eq!(a.0.len(), 64);
        assert_ne!(a.0, b.0);
    }
}
