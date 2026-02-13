//! gRPC → HTTP status code mapping.

use axum::http::StatusCode;

/// Return the canonical `SCREAMING_SNAKE_CASE` name for a gRPC status code.
///
/// Follows the [gRPC status code names](https://grpc.github.io/grpc/core/md_doc_statuscodes.html).
/// Useful for JSON error responses that include a machine-readable status field.
///
/// # Examples
///
/// ```
/// use tonic_rest::grpc_code_name;
///
/// assert_eq!(grpc_code_name(tonic::Code::NotFound), "NOT_FOUND");
/// assert_eq!(grpc_code_name(tonic::Code::InvalidArgument), "INVALID_ARGUMENT");
/// ```
#[must_use]
pub fn grpc_code_name(code: tonic::Code) -> &'static str {
    match code {
        tonic::Code::Ok => "OK",
        tonic::Code::Cancelled => "CANCELLED",
        tonic::Code::Unknown => "UNKNOWN",
        tonic::Code::InvalidArgument => "INVALID_ARGUMENT",
        tonic::Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        tonic::Code::NotFound => "NOT_FOUND",
        tonic::Code::AlreadyExists => "ALREADY_EXISTS",
        tonic::Code::PermissionDenied => "PERMISSION_DENIED",
        tonic::Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        tonic::Code::FailedPrecondition => "FAILED_PRECONDITION",
        tonic::Code::Aborted => "ABORTED",
        tonic::Code::OutOfRange => "OUT_OF_RANGE",
        tonic::Code::Unimplemented => "UNIMPLEMENTED",
        tonic::Code::Internal => "INTERNAL",
        tonic::Code::Unavailable => "UNAVAILABLE",
        tonic::Code::DataLoss => "DATA_LOSS",
        tonic::Code::Unauthenticated => "UNAUTHENTICATED",
    }
}

/// Map gRPC status codes to HTTP status codes.
///
/// Follows the [canonical mapping](https://grpc.github.io/grpc/core/md_doc_statuscodes.html).
///
/// # Examples
///
/// ```
/// use tonic_rest::grpc_to_http_status;
///
/// assert_eq!(grpc_to_http_status(tonic::Code::NotFound), axum::http::StatusCode::NOT_FOUND);
/// assert_eq!(grpc_to_http_status(tonic::Code::InvalidArgument), axum::http::StatusCode::BAD_REQUEST);
/// ```
#[must_use]
pub fn grpc_to_http_status(code: tonic::Code) -> StatusCode {
    match code {
        tonic::Code::Ok => StatusCode::OK,
        tonic::Code::Cancelled => StatusCode::REQUEST_TIMEOUT,
        tonic::Code::InvalidArgument | tonic::Code::OutOfRange => StatusCode::BAD_REQUEST,
        tonic::Code::Unauthenticated => StatusCode::UNAUTHORIZED,
        tonic::Code::PermissionDenied => StatusCode::FORBIDDEN,
        tonic::Code::NotFound => StatusCode::NOT_FOUND,
        tonic::Code::AlreadyExists | tonic::Code::Aborted => StatusCode::CONFLICT,
        tonic::Code::FailedPrecondition => StatusCode::PRECONDITION_FAILED,
        tonic::Code::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
        tonic::Code::Unimplemented => StatusCode::NOT_IMPLEMENTED,
        tonic::Code::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        tonic::Code::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
        tonic::Code::DataLoss | tonic::Code::Internal | tonic::Code::Unknown => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    /// Exhaustive test covering all 16 gRPC status codes.
    ///
    /// Validates the canonical mapping defined in
    /// <https://grpc.github.io/grpc/core/md_doc_statuscodes.html>.
    #[test]
    fn exhaustive_grpc_to_http_mapping() {
        let cases: &[(Code, StatusCode)] = &[
            (Code::Ok, StatusCode::OK),
            (Code::Cancelled, StatusCode::REQUEST_TIMEOUT),
            (Code::Unknown, StatusCode::INTERNAL_SERVER_ERROR),
            (Code::InvalidArgument, StatusCode::BAD_REQUEST),
            (Code::DeadlineExceeded, StatusCode::GATEWAY_TIMEOUT),
            (Code::NotFound, StatusCode::NOT_FOUND),
            (Code::AlreadyExists, StatusCode::CONFLICT),
            (Code::PermissionDenied, StatusCode::FORBIDDEN),
            (Code::ResourceExhausted, StatusCode::TOO_MANY_REQUESTS),
            (Code::FailedPrecondition, StatusCode::PRECONDITION_FAILED),
            (Code::Aborted, StatusCode::CONFLICT),
            (Code::OutOfRange, StatusCode::BAD_REQUEST),
            (Code::Unimplemented, StatusCode::NOT_IMPLEMENTED),
            (Code::Internal, StatusCode::INTERNAL_SERVER_ERROR),
            (Code::Unavailable, StatusCode::SERVICE_UNAVAILABLE),
            (Code::DataLoss, StatusCode::INTERNAL_SERVER_ERROR),
            (Code::Unauthenticated, StatusCode::UNAUTHORIZED),
        ];

        for (grpc_code, expected_http) in cases {
            assert_eq!(
                grpc_to_http_status(*grpc_code),
                *expected_http,
                "gRPC {grpc_code:?} should map to HTTP {expected_http}",
            );
        }

        // Verify we tested all 17 Code variants (16 error codes + Ok).
        assert_eq!(cases.len(), 17);
    }

    /// Exhaustive test covering all 17 gRPC status code names.
    #[test]
    fn exhaustive_grpc_code_name() {
        let cases: &[(Code, &str)] = &[
            (Code::Ok, "OK"),
            (Code::Cancelled, "CANCELLED"),
            (Code::Unknown, "UNKNOWN"),
            (Code::InvalidArgument, "INVALID_ARGUMENT"),
            (Code::DeadlineExceeded, "DEADLINE_EXCEEDED"),
            (Code::NotFound, "NOT_FOUND"),
            (Code::AlreadyExists, "ALREADY_EXISTS"),
            (Code::PermissionDenied, "PERMISSION_DENIED"),
            (Code::ResourceExhausted, "RESOURCE_EXHAUSTED"),
            (Code::FailedPrecondition, "FAILED_PRECONDITION"),
            (Code::Aborted, "ABORTED"),
            (Code::OutOfRange, "OUT_OF_RANGE"),
            (Code::Unimplemented, "UNIMPLEMENTED"),
            (Code::Internal, "INTERNAL"),
            (Code::Unavailable, "UNAVAILABLE"),
            (Code::DataLoss, "DATA_LOSS"),
            (Code::Unauthenticated, "UNAUTHENTICATED"),
        ];

        for (code, expected_name) in cases {
            assert_eq!(
                grpc_code_name(*code),
                *expected_name,
                "gRPC {code:?} should have name {expected_name}",
            );
        }

        assert_eq!(cases.len(), 17);
    }

    /// Codes that share the same HTTP status should be consistent.
    #[test]
    fn grouped_codes_share_http_status() {
        assert_eq!(
            grpc_to_http_status(Code::InvalidArgument),
            grpc_to_http_status(Code::OutOfRange),
        );
        assert_eq!(
            grpc_to_http_status(Code::AlreadyExists),
            grpc_to_http_status(Code::Aborted),
        );
        assert_eq!(
            grpc_to_http_status(Code::DataLoss),
            grpc_to_http_status(Code::Internal),
        );
        assert_eq!(
            grpc_to_http_status(Code::Internal),
            grpc_to_http_status(Code::Unknown),
        );
    }
}
