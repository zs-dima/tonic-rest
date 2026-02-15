//! REST error wrapper — converts [`tonic::Status`] to HTTP error responses.

use axum::extract::Json;
use axum::response::IntoResponse;

use super::status_map::{grpc_code_name, grpc_to_http_status};

/// REST error wrapper — converts [`tonic::Status`] to an HTTP error response.
///
/// Maps gRPC status codes to HTTP status codes and returns a JSON error body
/// following the [Google API error model](https://cloud.google.com/apis/design/errors):
///
/// ```json
/// {
///   "error": { "code": 400, "message": "...", "status": "INVALID_ARGUMENT" }
/// }
/// ```
///
/// # Response Format
///
/// The JSON shape is intentionally fixed to the Google API error convention.
/// This provides a consistent, well-documented error format across all generated
/// REST endpoints. The body wraps the error in an `"error"` object:
///
/// ```json
/// { "error": { "code": 404, "message": "...", "status": "NOT_FOUND" } }
/// ```
///
/// Note: SSE error events (via [`sse_error_event`](crate::sse_error_event)) use
/// the same `{"error": {...}}` format, ensuring a consistent error shape across
/// both HTTP JSON and SSE transports.
///
/// If you need a custom error shape, implement
/// [`axum::response::IntoResponse`] on your own error type and set the
/// `runtime_crate` config in `tonic-rest-build` to point to the module
/// containing your custom types.
///
/// # Constructing
///
/// Use [`From<tonic::Status>`] or [`RestError::new`]:
///
/// ```
/// # use tonic_rest::RestError;
/// let err = RestError::new(tonic::Status::not_found("gone"));
/// let err: RestError = tonic::Status::not_found("gone").into();
/// ```
///
/// # Examples
///
/// Convert a tonic status to an Axum-compatible HTTP response:
///
/// ```
/// use tonic_rest::RestError;
/// use axum::response::IntoResponse;
///
/// let err = RestError::new(tonic::Status::not_found("user not found"));
/// let response = err.into_response();
/// assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
/// ```
#[derive(Debug, Clone)]
pub struct RestError(tonic::Status);

impl std::fmt::Display for RestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", grpc_code_name(self.0.code()), self.0.message())
    }
}

impl std::error::Error for RestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl RestError {
    /// Create a new `RestError` from a [`tonic::Status`].
    #[must_use]
    pub const fn new(status: tonic::Status) -> Self {
        Self(status)
    }

    /// Returns a reference to the underlying [`tonic::Status`].
    #[must_use]
    pub const fn status(&self) -> &tonic::Status {
        &self.0
    }

    /// Consumes the `RestError` and returns the underlying [`tonic::Status`].
    #[must_use]
    pub fn into_status(self) -> tonic::Status {
        self.0
    }
}

impl From<tonic::Status> for RestError {
    fn from(status: tonic::Status) -> Self {
        Self(status)
    }
}

impl IntoResponse for RestError {
    fn into_response(self) -> axum::response::Response {
        let http_status = grpc_to_http_status(self.0.code());

        let body = serde_json::json!({
            "error": {
                "code": http_status.as_u16(),
                "message": self.0.message(),
                "status": grpc_code_name(self.0.code()),
            }
        });

        (http_status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    /// Parse the JSON error body from a `RestError` response.
    async fn error_body(status: tonic::Status) -> (axum::http::StatusCode, serde_json::Value) {
        let response = RestError::new(status).into_response();
        let http_status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (http_status, json)
    }

    #[tokio::test]
    async fn not_found_response() {
        let (status, json) = error_body(tonic::Status::not_found("user not found")).await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(json["error"]["code"], 404);
        assert_eq!(json["error"]["message"], "user not found");
        assert_eq!(json["error"]["status"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn invalid_argument_response() {
        let (status, json) = error_body(tonic::Status::invalid_argument("bad email")).await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], 400);
        assert_eq!(json["error"]["message"], "bad email");
        assert_eq!(json["error"]["status"], "INVALID_ARGUMENT");
    }

    #[tokio::test]
    async fn internal_error_response() {
        let (status, json) = error_body(tonic::Status::internal("db crashed")).await;
        assert_eq!(status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error"]["code"], 500);
        assert_eq!(json["error"]["message"], "db crashed");
        assert_eq!(json["error"]["status"], "INTERNAL");
    }

    #[tokio::test]
    async fn unauthenticated_response() {
        let (status, json) = error_body(tonic::Status::unauthenticated("token expired")).await;
        assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(json["error"]["code"], 401);
        assert_eq!(json["error"]["message"], "token expired");
        assert_eq!(json["error"]["status"], "UNAUTHENTICATED");
    }

    #[tokio::test]
    async fn permission_denied_response() {
        let (status, json) = error_body(tonic::Status::permission_denied("admin only")).await;
        assert_eq!(status, axum::http::StatusCode::FORBIDDEN);
        assert_eq!(json["error"]["code"], 403);
        assert_eq!(json["error"]["message"], "admin only");
        assert_eq!(json["error"]["status"], "PERMISSION_DENIED");
    }

    #[tokio::test]
    async fn empty_message_response() {
        let (status, json) = error_body(tonic::Status::internal("")).await;
        assert_eq!(status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error"]["message"], "");
    }

    #[test]
    fn from_tonic_status() {
        let status = tonic::Status::not_found("gone");
        let err = RestError::from(status);
        assert_eq!(err.status().code(), tonic::Code::NotFound);
        assert_eq!(err.status().message(), "gone");
    }

    #[test]
    fn display_format() {
        let err = RestError::new(tonic::Status::not_found("user not found"));
        assert_eq!(err.to_string(), "NOT_FOUND: user not found");
    }

    #[test]
    fn display_empty_message() {
        let err = RestError::new(tonic::Status::internal(""));
        assert_eq!(err.to_string(), "INTERNAL: ");
    }

    #[test]
    fn debug_format() {
        let err = RestError::new(tonic::Status::not_found("gone"));
        let debug = format!("{err:?}");
        assert!(debug.contains("RestError"), "missing type name: {debug}");
    }

    #[test]
    fn error_source_is_tonic_status() {
        use std::error::Error;
        let err = RestError::new(tonic::Status::internal("boom"));
        let source = err.source().expect("should have a source");
        assert!(
            source.to_string().contains("boom"),
            "source should contain message: {source}",
        );
    }

    #[tokio::test]
    async fn response_content_type_is_json() {
        let response = RestError::new(tonic::Status::not_found("x")).into_response();
        let content_type = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            content_type.contains("application/json"),
            "expected JSON content-type, got: {content_type}",
        );
    }

    #[test]
    fn status_accessor_returns_inner() {
        let err = RestError::new(tonic::Status::not_found("gone"));
        assert_eq!(err.status().code(), tonic::Code::NotFound);
        assert_eq!(err.status().message(), "gone");
    }

    #[test]
    fn into_status_consumes_and_returns_inner() {
        let err = RestError::new(tonic::Status::permission_denied("nope"));
        let status = err.into_status();
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert_eq!(status.message(), "nope");
    }
}
