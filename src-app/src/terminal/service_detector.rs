//! Detect local dev-server metadata (port, URL, framework label) from a
//! single line of PTY output.
//!
//! Called by `TerminalState::scan_output` after each output batch.  The
//! returned `ServiceInfo` is surfaced in the workspace sidebar.
//!
//! Keep the matchers string-based and allocation-light: this runs on every
//! terminal write batch, on the GPUI main thread.

/// Metadata about a detected service (server listening on a port).
/// Enriches the bare port number from `/proc/net/tcp` with human-readable info.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceInfo {
    pub port: u16,
    pub url: Option<String>,
    pub label: Option<String>,
    /// True for frontend dev servers (Next.js, Vite, Nuxt) — clickable in sidebar.
    pub is_frontend: bool,
}

/// Parse a terminal output line for local server URL patterns.
/// Derived from VS Code's UrlFinder — anchors on localhost/127.0.0.1/0.0.0.0.
pub(super) fn parse_service_line(line: &str) -> Option<ServiceInfo> {
    let port = extract_local_port(line)?;
    if port == 0 {
        return None;
    }
    let url = extract_url(line);
    let (label, is_frontend) = detect_framework(line);
    Some(ServiceInfo {
        port,
        url,
        label,
        is_frontend,
    })
}

/// Extract a port number from localhost:PORT, 127.0.0.1:PORT, or 0.0.0.0:PORT patterns.
/// Also handles Python's `http.server` format: "HTTP on 127.0.0.1 port 8000".
fn extract_local_port(line: &str) -> Option<u16> {
    for anchor in ["localhost:", "127.0.0.1:", "0.0.0.0:"] {
        if let Some(idx) = line.find(anchor) {
            let after = &line[idx + anchor.len()..];
            let port_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(port) = port_str.parse::<u16>() {
                return Some(port);
            }
        }
    }
    // Python http.server: "HTTP on 127.0.0.1 port 8000"
    if let Some(idx) = line.find(" port ")
        && (line.contains("127.0.0.1") || line.contains("0.0.0.0"))
    {
        let after = &line[idx + 6..];
        let port_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(port) = port_str.parse::<u16>() {
            return Some(port);
        }
    }
    None
}

/// Extract a full URL (http:// or https://) from a terminal line.
fn extract_url(line: &str) -> Option<String> {
    for scheme in ["https://", "http://"] {
        if let Some(start) = line.find(scheme) {
            let url: String = line[start..]
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != ')' && *c != '"' && *c != '\'')
                .collect();
            if url.len() > scheme.len() {
                return Some(url);
            }
        }
    }
    None
}

/// Detect the framework/server name from keywords in the terminal line.
/// Returns `(label, is_frontend)` — frontend frameworks get clickable URLs in the sidebar.
/// Uses word-boundary matching to avoid false positives (e.g. "origin" matching "gin").
pub(super) fn detect_framework(line: &str) -> (Option<String>, bool) {
    // (keyword, display_label, is_frontend)
    const FRAMEWORKS: &[(&str, &str, bool)] = &[
        ("next.js", "Next.js", true),
        ("next dev", "Next.js", true),
        ("turbopack", "Next.js", true),
        ("vite", "Vite", true),
        ("nuxt", "Nuxt", true),
        ("remix", "Remix", true),
        ("astro", "Astro", true),
        ("webpack-dev-server", "Webpack", true),
        ("angular", "Angular", true),
        ("express", "Express", false),
        ("fastify", "Fastify", false),
        ("uvicorn", "uvicorn", false),
        ("flask", "Flask", false),
        ("django", "Django", false),
        ("rocket", "Rocket", false),
        ("actix-web", "Actix", false),
        ("axum", "Axum", false),
        ("gin-gonic", "Gin", false),
        ("fiber", "Fiber", false),
        ("puma", "Puma", false),
        ("tomcat", "Tomcat", false),
        ("laravel", "Laravel", false),
        ("spring boot", "Spring", false),
    ];
    let lower = line.to_lowercase();
    for (key, label, frontend) in FRAMEWORKS {
        if lower.contains(key) {
            return (Some(label.to_string()), *frontend);
        }
    }
    (None, false)
}
