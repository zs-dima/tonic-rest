//! Tonic request builder — bridges Axum HTTP requests to [`tonic::Request`].

use axum::http::{HeaderMap, HeaderName};
use tonic::Request;

/// HTTP headers forwarded from Axum to tonic metadata for client context.
///
/// Contains standard headers for authentication and client identification.
/// Use [`build_tonic_request_with_headers`] for custom header sets,
/// or include [`CLOUDFLARE_HEADERS`] if your service runs behind Cloudflare.
pub const FORWARDED_HEADERS: &[&str] = &[
    "authorization",
    "user-agent",
    "x-forwarded-for",
    "x-real-ip",
];

/// Cloudflare-specific headers for client IP resolution.
///
/// Include these alongside [`FORWARDED_HEADERS`] if your service runs
/// behind Cloudflare:
///
/// ```
/// # use tonic_rest::{FORWARDED_HEADERS, CLOUDFLARE_HEADERS};
/// let all_headers: Vec<&str> = FORWARDED_HEADERS.iter()
///     .chain(CLOUDFLARE_HEADERS.iter())
///     .copied()
///     .collect();
/// ```
pub const CLOUDFLARE_HEADERS: &[&str] = &["cf-connecting-ip"];

/// Return [`FORWARDED_HEADERS`] as typed [`HeaderName`] values.
///
/// Useful when working with `HeaderMap` APIs that require `HeaderName`
/// instead of string slices.
///
/// # Example
///
/// ```
/// use tonic_rest::forwarded_header_names;
///
/// let names = forwarded_header_names();
/// assert_eq!(names.len(), 4);
/// ```
#[must_use]
pub fn forwarded_header_names() -> [HeaderName; 4] {
    [
        HeaderName::from_static("authorization"),
        HeaderName::from_static("user-agent"),
        HeaderName::from_static("x-forwarded-for"),
        HeaderName::from_static("x-real-ip"),
    ]
}

/// Return [`CLOUDFLARE_HEADERS`] as typed [`HeaderName`] values.
///
/// # Example
///
/// ```
/// use tonic_rest::cloudflare_header_names;
///
/// let names = cloudflare_header_names();
/// assert_eq!(names.len(), 1);
/// ```
#[must_use]
pub fn cloudflare_header_names() -> [HeaderName; 1] {
    [HeaderName::from_static("cf-connecting-ip")]
}

/// Build a [`tonic::Request`] without an extension type.
///
/// Convenience wrapper around [`build_tonic_request`] for endpoints that
/// don't use extension-based auth. Avoids the turbofish syntax
/// `build_tonic_request::<_, ()>(body, &headers, None)`.
///
/// # Examples
///
/// ```
/// use axum::http::HeaderMap;
/// use tonic_rest::build_tonic_request_simple;
///
/// let mut headers = HeaderMap::new();
/// headers.insert("authorization", "Bearer token".parse().unwrap());
///
/// let req = build_tonic_request_simple("body", &headers);
/// assert_eq!(req.metadata().get("authorization").unwrap(), "Bearer token");
/// ```
pub fn build_tonic_request_simple<T>(body: T, headers: &HeaderMap) -> Request<T> {
    build_tonic_request::<T, ()>(body, headers, None)
}

/// Build a [`tonic::Request`] from a body, HTTP headers, and an optional extension.
///
/// This bridges the Axum → Tonic boundary:
/// - Forwards an extension value (typically auth info from middleware) into
///   tonic request extensions so service methods can access it
/// - Copies relevant HTTP headers to tonic metadata for authentication,
///   client identification, and IP resolution
///
/// # Type Parameters
///
/// - `T` — Request body type (proto message)
/// - `E` — Extension type (e.g., `AuthInfo`). Use `()` for no extension.
///
/// # Examples
///
/// ```
/// use axum::http::HeaderMap;
/// use tonic_rest::build_tonic_request;
///
/// let mut headers = HeaderMap::new();
/// headers.insert("authorization", "Bearer token".parse().unwrap());
///
/// // Without extension:
/// let req = build_tonic_request::<_, ()>("body", &headers, None);
/// assert_eq!(req.metadata().get("authorization").unwrap(), "Bearer token");
///
/// // With extension:
/// let req = build_tonic_request("body", &headers, Some(42u32));
/// assert_eq!(req.extensions().get::<u32>(), Some(&42));
/// ```
pub fn build_tonic_request<T, E>(body: T, headers: &HeaderMap, extension: Option<E>) -> Request<T>
where
    E: Clone + Send + Sync + 'static,
{
    build_tonic_request_with_headers(body, headers, extension, FORWARDED_HEADERS)
}

/// Build a [`tonic::Request`] with a custom set of forwarded headers.
///
/// Like [`build_tonic_request`] but lets you control which HTTP headers
/// are copied to tonic metadata.
///
/// # Arguments
///
/// * `body` — Request body (proto message)
/// * `headers` — Incoming HTTP headers from Axum
/// * `extension` — Optional extension value (e.g., auth info from middleware)
/// * `forwarded_headers` — Header names to copy to tonic metadata
///
/// # Examples
///
/// ```
/// use axum::http::HeaderMap;
/// use tonic_rest::build_tonic_request_with_headers;
///
/// let mut headers = HeaderMap::new();
/// headers.insert("x-custom", "value".parse().unwrap());
///
/// let req = build_tonic_request_with_headers::<_, ()>(
///     "body", &headers, None, &["x-custom"]
/// );
/// assert_eq!(req.metadata().get("x-custom").unwrap(), "value");
/// ```
pub fn build_tonic_request_with_headers<T, E>(
    body: T,
    headers: &HeaderMap,
    extension: Option<E>,
    forwarded_headers: &[&str],
) -> Request<T>
where
    E: Clone + Send + Sync + 'static,
{
    let mut req = Request::new(body);

    // Forward the extension (e.g., auth info from middleware) so service
    // methods can access it via request extensions.
    if let Some(ext) = extension {
        req.extensions_mut().insert(ext);
    }

    // Copy relevant HTTP headers to tonic metadata.
    // Silently skip values that can't be parsed (non-ASCII, control chars)
    // rather than panicking — malformed client headers shouldn't crash the server.
    let metadata = req.metadata_mut();
    for &name in forwarded_headers {
        let Some(val) = headers.get(name).and_then(|v| v.to_str().ok()) else {
            continue;
        };
        let Ok(key) = name.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>() else {
            continue;
        };
        if let Ok(parsed) = val.parse() {
            metadata.insert(key, parsed);
        }
    }

    req
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_headers_contains_expected_entries() {
        assert_eq!(FORWARDED_HEADERS.len(), 4);
        assert!(FORWARDED_HEADERS.contains(&"authorization"));
        assert!(FORWARDED_HEADERS.contains(&"user-agent"));
        assert!(FORWARDED_HEADERS.contains(&"x-forwarded-for"));
        assert!(FORWARDED_HEADERS.contains(&"x-real-ip"));
    }

    #[test]
    fn cloudflare_headers_contains_cf_connecting_ip() {
        assert_eq!(CLOUDFLARE_HEADERS.len(), 1);
        assert!(CLOUDFLARE_HEADERS.contains(&"cf-connecting-ip"));
    }

    #[test]
    fn forwarded_header_names_matches_const() {
        let names = forwarded_header_names();
        assert_eq!(names.len(), FORWARDED_HEADERS.len());
        for (name, &expected) in names.iter().zip(FORWARDED_HEADERS.iter()) {
            assert_eq!(name.as_str(), expected);
        }
    }

    #[test]
    fn cloudflare_header_names_matches_const() {
        let names = cloudflare_header_names();
        assert_eq!(names.len(), CLOUDFLARE_HEADERS.len());
        for (name, &expected) in names.iter().zip(CLOUDFLARE_HEADERS.iter()) {
            assert_eq!(name.as_str(), expected);
        }
    }

    #[test]
    fn no_auth_no_headers() {
        let headers = HeaderMap::new();
        let req = build_tonic_request::<_, ()>("body", &headers, None);
        assert_eq!(*req.get_ref(), "body");
        assert!(req.extensions().get::<()>().is_none());
        assert!(req.metadata().is_empty());
    }

    #[test]
    fn auth_inserted_into_extensions() {
        #[derive(Clone, Debug, PartialEq)]
        struct Auth(String);

        let headers = HeaderMap::new();
        let req = build_tonic_request("body", &headers, Some(Auth("user1".to_string())));
        assert_eq!(
            req.extensions().get::<Auth>(),
            Some(&Auth("user1".to_string())),
        );
    }

    #[test]
    fn forwards_known_headers_to_metadata() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer tok".parse().unwrap());
        headers.insert("user-agent", "TestClient/1".parse().unwrap());
        headers.insert("x-forwarded-for", "1.2.3.4".parse().unwrap());
        headers.insert("x-real-ip", "5.6.7.8".parse().unwrap());

        let req = build_tonic_request::<_, ()>("b", &headers, None);
        let meta = req.metadata();
        assert_eq!(meta.get("authorization").unwrap(), "Bearer tok");
        assert_eq!(meta.get("user-agent").unwrap(), "TestClient/1");
        assert_eq!(meta.get("x-forwarded-for").unwrap(), "1.2.3.4");
        assert_eq!(meta.get("x-real-ip").unwrap(), "5.6.7.8");
    }

    #[test]
    fn cloudflare_headers_forwarded_with_custom_list() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "9.10.11.12".parse().unwrap());
        headers.insert("authorization", "Bearer tok".parse().unwrap());

        let all: Vec<&str> = FORWARDED_HEADERS
            .iter()
            .chain(CLOUDFLARE_HEADERS.iter())
            .copied()
            .collect();
        let req = build_tonic_request_with_headers::<_, ()>("b", &headers, None, &all);
        let meta = req.metadata();
        assert_eq!(meta.get("cf-connecting-ip").unwrap(), "9.10.11.12");
        assert_eq!(meta.get("authorization").unwrap(), "Bearer tok");
    }

    #[test]
    fn skips_unknown_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer tok".parse().unwrap());
        headers.insert("x-custom-header", "custom".parse().unwrap());
        headers.insert("x-request-id", "abc-123".parse().unwrap());

        let req = build_tonic_request::<_, ()>("b", &headers, None);
        let meta = req.metadata();
        assert_eq!(meta.get("authorization").unwrap(), "Bearer tok");
        assert!(meta.get("x-custom-header").is_none());
        assert!(meta.get("x-request-id").is_none());
    }

    #[test]
    fn custom_header_list() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer tok".parse().unwrap());
        headers.insert("x-custom", "value".parse().unwrap());

        let req = build_tonic_request_with_headers::<_, ()>("b", &headers, None, &["x-custom"]);
        let meta = req.metadata();
        assert_eq!(meta.get("x-custom").unwrap(), "value");
        // `authorization` is NOT forwarded because it's not in the custom list.
        assert!(meta.get("authorization").is_none());
    }

    #[test]
    fn empty_custom_header_list_forwards_nothing() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer tok".parse().unwrap());

        let req = build_tonic_request_with_headers::<_, ()>("b", &headers, None, &[]);
        assert!(req.metadata().is_empty());
    }

    #[test]
    fn missing_header_values_silently_skipped() {
        let headers = HeaderMap::new(); // no headers at all
        let req = build_tonic_request::<_, ()>("b", &headers, None);
        assert!(req.metadata().is_empty());
    }

    #[test]
    fn auth_and_headers_combined() {
        #[derive(Clone, Debug, PartialEq)]
        struct Auth(u32);

        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "Bot/2".parse().unwrap());

        let req = build_tonic_request("msg", &headers, Some(Auth(42)));
        assert_eq!(req.extensions().get::<Auth>(), Some(&Auth(42)));
        assert_eq!(req.metadata().get("user-agent").unwrap(), "Bot/2");
    }
}
