//! The exported frontend. Next's static export writes one `<route>.html` per
//! page and a `<route>/` directory of client data beside it, so `/mail` must be
//! served `mail.html` directly: `ServeDir` alone redirected it into the data
//! directory and fell back to the login page, which navigated back to `/mail`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Uri};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

pub fn router(static_dir: &str) -> Router {
    let dir = PathBuf::from(static_dir);
    let serve = ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html")));
    Router::new()
        .fallback_service(serve)
        .layer(middleware::from_fn_with_state(Arc::new(dir), route_pages))
}

/// `/mail`, `/mail/` → `/mail.html` when that file exists; HTML is marked
/// `no-cache` so a browser revalidates the page shell after every deploy
/// instead of running an old build against new data.
async fn route_pages(State(dir): State<Arc<PathBuf>>, mut req: Request, next: Next) -> Response {
    if let Some(uri) = html_page_for(&dir, req.uri()) {
        *req.uri_mut() = uri;
    }
    let mut res = next.run(req).await;
    let is_html = res
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(b"text/html"));
    if is_html {
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    res
}

fn html_page_for(dir: &Path, uri: &Uri) -> Option<Uri> {
    let route = uri.path().trim_matches('/');
    if route.is_empty() || route.contains("..") || Path::new(route).extension().is_some() {
        return None;
    }
    // ponytail: one stat per extensionless request; cache the page list if this
    // ever shows up in latency.
    if !dir.join(format!("{route}.html")).is_file() {
        return None;
    }
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    format!("/{route}.html{query}").parse().ok()
}
