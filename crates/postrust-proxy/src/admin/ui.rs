//! HTML admin UI for proxy management.

use crate::ProxyState;
use axum::{extract::State, response::Html, routing::get, Router};
use std::sync::Arc;

/// Create the admin UI router.
pub fn admin_ui_router() -> Router<Arc<ProxyState>> {
    Router::new()
        .route("/", get(proxy_dashboard))
        .route("/routes", get(routes_page))
        .route("/upstreams", get(upstreams_page))
        .route("/health", get(health_page))
        .route("/certificates", get(certificates_page))
}

async fn proxy_dashboard(State(state): State<Arc<ProxyState>>) -> Html<String> {
    let config = state.config.read().await;

    let backends_count: usize = config.upstreams.iter().map(|u| u.backends.len()).sum();

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Proxy Dashboard - Postrust</title>
    <style>
        body {{ font-family: system-ui, sans-serif; margin: 0; padding: 20px; background: #f5f5f5; }}
        .container {{ max-width: 1200px; margin: 0 auto; }}
        h1 {{ color: #333; }}
        .stats {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 20px; margin: 20px 0; }}
        .stat-card {{ background: white; padding: 20px; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }}
        .stat-card h3 {{ margin: 0 0 10px 0; color: #666; font-size: 14px; }}
        .stat-card .value {{ font-size: 36px; font-weight: bold; color: #333; }}
        .nav {{ margin: 20px 0; }}
        .nav a {{ display: inline-block; padding: 10px 20px; background: #007bff; color: white; text-decoration: none; border-radius: 4px; margin-right: 10px; }}
        .nav a:hover {{ background: #0056b3; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Proxy Dashboard</h1>

        <div class="nav">
            <a href="/admin/proxy/routes">Routes</a>
            <a href="/admin/proxy/upstreams">Upstreams</a>
            <a href="/admin/proxy/health">Health</a>
            <a href="/admin/proxy/certificates">Certificates</a>
        </div>

        <div class="stats">
            <div class="stat-card">
                <h3>Routes</h3>
                <div class="value">{}</div>
            </div>
            <div class="stat-card">
                <h3>Upstreams</h3>
                <div class="value">{}</div>
            </div>
            <div class="stat-card">
                <h3>Backends</h3>
                <div class="value">{}</div>
            </div>
        </div>
    </div>
</body>
</html>"#,
        config.routes.len(),
        config.upstreams.len(),
        backends_count,
    );

    Html(html)
}

async fn routes_page(State(state): State<Arc<ProxyState>>) -> Html<String> {
    let config = state.config.read().await;

    let routes_html: String = config
        .routes
        .iter()
        .map(|r| {
            format!(
                r#"<tr>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
            </tr>"#,
                r.id.map(|id| id.to_string()).unwrap_or_default(),
                r.name,
                r.match_.host.as_deref().unwrap_or("*"),
                r.match_.path.as_deref().unwrap_or("/"),
                r.priority,
            )
        })
        .collect();

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Routes - Postrust Proxy</title>
    <style>
        body {{ font-family: system-ui, sans-serif; margin: 0; padding: 20px; background: #f5f5f5; }}
        .container {{ max-width: 1200px; margin: 0 auto; }}
        h1 {{ color: #333; }}
        table {{ width: 100%; border-collapse: collapse; background: white; border-radius: 8px; overflow: hidden; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }}
        th, td {{ padding: 12px 15px; text-align: left; border-bottom: 1px solid #eee; }}
        th {{ background: #f8f9fa; font-weight: 600; }}
        .nav {{ margin: 20px 0; }}
        .nav a {{ display: inline-block; padding: 10px 20px; background: #6c757d; color: white; text-decoration: none; border-radius: 4px; margin-right: 10px; }}
        .nav a:hover {{ background: #5a6268; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Routes</h1>

        <div class="nav">
            <a href="/admin/proxy/">Back to Dashboard</a>
        </div>

        <table>
            <thead>
                <tr>
                    <th>ID</th>
                    <th>Name</th>
                    <th>Host</th>
                    <th>Path Prefix</th>
                    <th>Priority</th>
                </tr>
            </thead>
            <tbody>
                {}
            </tbody>
        </table>
    </div>
</body>
</html>"#,
        routes_html
    );

    Html(html)
}

async fn upstreams_page(State(state): State<Arc<ProxyState>>) -> Html<String> {
    let config = state.config.read().await;

    let upstreams_html: String = config
        .upstreams
        .iter()
        .map(|u| {
            format!(
                r#"<tr>
                <td>{}</td>
                <td>{}</td>
                <td>{:?}</td>
                <td>{}</td>
            </tr>"#,
                u.id.map(|id| id.to_string()).unwrap_or_default(),
                u.name,
                u.lb_strategy,
                u.backends.len(),
            )
        })
        .collect();

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Upstreams - Postrust Proxy</title>
    <style>
        body {{ font-family: system-ui, sans-serif; margin: 0; padding: 20px; background: #f5f5f5; }}
        .container {{ max-width: 1200px; margin: 0 auto; }}
        h1 {{ color: #333; }}
        table {{ width: 100%; border-collapse: collapse; background: white; border-radius: 8px; overflow: hidden; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }}
        th, td {{ padding: 12px 15px; text-align: left; border-bottom: 1px solid #eee; }}
        th {{ background: #f8f9fa; font-weight: 600; }}
        .nav {{ margin: 20px 0; }}
        .nav a {{ display: inline-block; padding: 10px 20px; background: #6c757d; color: white; text-decoration: none; border-radius: 4px; margin-right: 10px; }}
        .nav a:hover {{ background: #5a6268; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Upstreams</h1>

        <div class="nav">
            <a href="/admin/proxy/">Back to Dashboard</a>
        </div>

        <table>
            <thead>
                <tr>
                    <th>ID</th>
                    <th>Name</th>
                    <th>Load Balance</th>
                    <th>Backends</th>
                </tr>
            </thead>
            <tbody>
                {}
            </tbody>
        </table>
    </div>
</body>
</html>"#,
        upstreams_html
    );

    Html(html)
}

async fn health_page(State(state): State<Arc<ProxyState>>) -> Html<String> {
    let config = state.config.read().await;

    let mut health_html = String::new();
    for upstream in &config.upstreams {
        for backend in &upstream.backends {
            let (status, status_class) = if let Some(id) = backend.id {
                if state.health_checker.is_healthy(id) {
                    ("Healthy", "healthy")
                } else {
                    ("Unhealthy", "unhealthy")
                }
            } else {
                ("Unknown", "unknown")
            };

            health_html.push_str(&format!(
                r#"<tr>
                    <td>{}</td>
                    <td>{}</td>
                    <td>{}</td>
                    <td class="{}">{}</td>
                </tr>"#,
                upstream.name, backend.address, backend.scheme, status_class, status,
            ));
        }
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Health Status - Postrust Proxy</title>
    <style>
        body {{ font-family: system-ui, sans-serif; margin: 0; padding: 20px; background: #f5f5f5; }}
        .container {{ max-width: 1200px; margin: 0 auto; }}
        h1 {{ color: #333; }}
        table {{ width: 100%; border-collapse: collapse; background: white; border-radius: 8px; overflow: hidden; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }}
        th, td {{ padding: 12px 15px; text-align: left; border-bottom: 1px solid #eee; }}
        th {{ background: #f8f9fa; font-weight: 600; }}
        .healthy {{ color: #28a745; font-weight: bold; }}
        .unhealthy {{ color: #dc3545; font-weight: bold; }}
        .unknown {{ color: #6c757d; }}
        .nav {{ margin: 20px 0; }}
        .nav a {{ display: inline-block; padding: 10px 20px; background: #6c757d; color: white; text-decoration: none; border-radius: 4px; margin-right: 10px; }}
        .nav a:hover {{ background: #5a6268; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Backend Health Status</h1>

        <div class="nav">
            <a href="/admin/proxy/">Back to Dashboard</a>
        </div>

        <table>
            <thead>
                <tr>
                    <th>Upstream</th>
                    <th>Backend Address</th>
                    <th>Scheme</th>
                    <th>Status</th>
                </tr>
            </thead>
            <tbody>
                {}
            </tbody>
        </table>
    </div>
</body>
</html>"#,
        health_html
    );

    Html(html)
}

async fn certificates_page(State(_state): State<Arc<ProxyState>>) -> Html<String> {
    // TODO: Integrate with CertificateStore
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>Certificates - Postrust Proxy</title>
    <style>
        body { font-family: system-ui, sans-serif; margin: 0; padding: 20px; background: #f5f5f5; }
        .container { max-width: 1200px; margin: 0 auto; }
        h1 { color: #333; }
        .placeholder { background: white; padding: 40px; text-align: center; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); color: #666; }
        .nav { margin: 20px 0; }
        .nav a { display: inline-block; padding: 10px 20px; background: #6c757d; color: white; text-decoration: none; border-radius: 4px; margin-right: 10px; }
        .nav a:hover { background: #5a6268; }
    </style>
</head>
<body>
    <div class="container">
        <h1>SSL/TLS Certificates</h1>

        <div class="nav">
            <a href="/admin/proxy/">Back to Dashboard</a>
        </div>

        <div class="placeholder">
            <p>Certificate management will be available here.</p>
            <p>Certificates are automatically managed via Let's Encrypt ACME.</p>
        </div>
    </div>
</body>
</html>"#;

    Html(html.to_string())
}
