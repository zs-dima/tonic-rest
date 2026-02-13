//! SSE error event formatting.

use axum::response::sse::Event;

use super::status_map::grpc_to_http_status;

/// Build a structured SSE error event from a [`tonic::Status`].
///
/// Returns a JSON error object so SSE clients can distinguish error types
/// (auth failure vs server error, etc.):
///
/// ```text
/// event: error
/// data: {"code":401,"status":"UNAUTHENTICATED","message":"..."}
/// ```
///
/// Note: The JSON body uses a flat structure (no `"error"` wrapper), unlike
/// [`RestError`](crate::RestError) which wraps in `{"error": {...}}`. The SSE
/// event type field (`event: error`) already distinguishes errors from data events.
///
/// # Examples
///
/// ```
/// use tonic_rest::sse_error_event;
///
/// let event = sse_error_event(&tonic::Status::unauthenticated("token expired"));
/// // The event will have `event: error` type and JSON data with code 401
/// ```
pub fn sse_error_event(status: &tonic::Status) -> Event {
    let http_code = grpc_to_http_status(status.code());
    let body = serde_json::json!({
        "code": http_code.as_u16(),
        "status": super::status_map::grpc_code_name(status.code()),
        "message": status.message(),
    });
    Event::default()
        .event("error")
        .json_data(&body)
        .unwrap_or_else(|_| Event::default().event("error").data(status.message()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    use axum::response::sse::Sse;
    use axum::response::IntoResponse;
    use futures::stream;
    use http_body_util::BodyExt;

    /// Render a single SSE event to its text/event-stream representation.
    async fn render_event(event: Event) -> String {
        let sse = Sse::new(stream::once(async move { Ok::<_, Infallible>(event) }));
        let response = sse.into_response();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn error_event_not_found() {
        let status = tonic::Status::not_found("item gone");
        let text = render_event(sse_error_event(&status)).await;

        assert!(text.contains("event: error"), "missing event type: {text}");
        assert!(text.contains("\"code\":404"), "missing HTTP code: {text}");
        assert!(
            text.contains("\"status\":\"NOT_FOUND\""),
            "missing gRPC status: {text}",
        );
        assert!(
            text.contains("\"message\":\"item gone\""),
            "missing message: {text}",
        );
    }

    #[tokio::test]
    async fn error_event_unauthenticated() {
        let status = tonic::Status::unauthenticated("token expired");
        let text = render_event(sse_error_event(&status)).await;

        assert!(text.contains("event: error"), "missing event type: {text}");
        assert!(text.contains("\"code\":401"), "missing HTTP code: {text}");
        assert!(
            text.contains("\"status\":\"UNAUTHENTICATED\""),
            "missing gRPC status: {text}",
        );
    }

    #[tokio::test]
    async fn error_event_internal() {
        let status = tonic::Status::internal("oops");
        let text = render_event(sse_error_event(&status)).await;

        assert!(text.contains("\"code\":500"), "missing HTTP code: {text}");
        assert!(
            text.contains("\"status\":\"INTERNAL\""),
            "missing gRPC status: {text}",
        );
    }

    #[tokio::test]
    async fn error_event_empty_message() {
        let status = tonic::Status::internal("");
        let text = render_event(sse_error_event(&status)).await;

        assert!(text.contains("event: error"), "missing event type: {text}");
        assert!(
            text.contains("\"message\":\"\""),
            "missing empty message: {text}",
        );
    }

    /// Verify that the SSE response has the correct content-type header.
    #[tokio::test]
    async fn sse_content_type() {
        let event = sse_error_event(&tonic::Status::ok("ok"));
        let sse = Sse::new(stream::once(async move { Ok::<_, Infallible>(event) }));
        let response = sse.into_response();
        let ct = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            ct.contains("text/event-stream"),
            "expected text/event-stream, got: {ct}",
        );
    }
}
