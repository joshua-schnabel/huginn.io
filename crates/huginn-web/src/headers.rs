//! Security response headers for both HTTP listeners.
//!
//! Hand-rolled with `axum::middleware::from_fn` rather than `tower-http`'s
//! `SetResponseHeaderLayer`: adding a direct dependency needs approval
//! (AGENTS.md §3), and this is a dozen lines of constant headers.

use axum::extract::Request;
use axum::http::header::{HeaderMap, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

/// Content-Security-Policy for the debug UI.
///
/// `index.html` carries no inline `<script>` or `<style>` and loads nothing
/// off-host, so the strictest useful policy applies: deny everything, then
/// re-allow same-origin script/style (`/assets/*`) and same-origin fetch/SSE
/// (`/metrics/latest`, `/events`). `script-src 'self'` *without* `unsafe-inline`
/// is the part that matters: it makes an inline event handler — the `onerror=`
/// in an injected tag — inert, which is why this sits behind the escaping in
/// `app.js` as defence in depth rather than instead of it.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
     connect-src 'self'; img-src 'none'; base-uri 'none'; form-action 'none'; \
     frame-ancestors 'none'";

/// Headers set on every response from both listeners.
///
/// `nosniff` matters most for `/metrics/latest` and `/metrics`: both echo
/// operator- and remote-supplied strings inside a non-HTML content type, and a
/// sniffing browser asked to render one of them as HTML is exactly the case the
/// header exists to stop. `frame-ancestors`/`X-Frame-Options` keep the UI out of
/// a foreign frame, and `no-referrer` keeps probe URLs out of `Referer`.
const SECURITY_HEADERS: [(&str, &str); 5] = [
    ("content-security-policy", CSP),
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "no-referrer"),
    // Probe results are live operational data — a cached copy is both stale and
    // one more place the monitored inventory sits around.
    ("cache-control", "no-store"),
];

/// Write [`SECURITY_HEADERS`] into `headers`, replacing any existing value.
pub fn apply_security_headers(headers: &mut HeaderMap) {
    for (name, value) in SECURITY_HEADERS {
        // Both sides are compile-time constants, so neither conversion can fail.
        // Skipping on error rather than unwrapping keeps an unreachable case out
        // of the panic budget (AGENTS.md §6).
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(name, value);
        }
    }
}

/// Middleware that attaches [`SECURITY_HEADERS`] to every response.
pub async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    apply_security_headers(response.headers_mut());
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied() -> HeaderMap {
        let mut headers = HeaderMap::new();
        apply_security_headers(&mut headers);
        headers
    }

    fn value(headers: &HeaderMap, name: &str) -> String {
        headers
            .get(name)
            .unwrap_or_else(|| panic!("header '{name}' missing"))
            .to_str()
            .expect("header value is ASCII")
            .to_string()
    }

    #[test]
    fn csp_denies_everything_but_same_origin_script_and_style() {
        let csp = value(&applied(), "content-security-policy");
        assert!(csp.contains("default-src 'none'"), "got: {csp}");
        assert!(csp.contains("script-src 'self'"), "got: {csp}");
        assert!(csp.contains("style-src 'self'"), "got: {csp}");
        assert!(csp.contains("connect-src 'self'"), "got: {csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "got: {csp}");
    }

    /// `unsafe-inline` would re-enable the injected `onerror=` handler that this
    /// policy exists to neutralise.
    #[test]
    fn csp_never_allows_inline_script() {
        let csp = value(&applied(), "content-security-policy");
        assert!(!csp.contains("unsafe-inline"), "got: {csp}");
        assert!(!csp.contains("unsafe-eval"), "got: {csp}");
    }

    #[test]
    fn hardening_headers_are_set() {
        let headers = applied();
        assert_eq!(value(&headers, "x-content-type-options"), "nosniff");
        assert_eq!(value(&headers, "x-frame-options"), "DENY");
        assert_eq!(value(&headers, "referrer-policy"), "no-referrer");
        assert_eq!(value(&headers, "cache-control"), "no-store");
    }

    /// The middleware overwrites rather than appends: two `cache-control` values
    /// on one response is ambiguous, and the weaker one might win.
    #[test]
    fn existing_values_are_replaced_not_appended() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "cache-control",
            HeaderValue::from_static("public,max-age=60"),
        );
        apply_security_headers(&mut headers);
        assert_eq!(headers.get_all("cache-control").iter().count(), 1);
        assert_eq!(value(&headers, "cache-control"), "no-store");
    }
}
