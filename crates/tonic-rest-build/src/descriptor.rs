//! Minimal protobuf descriptor types with `google.api.http` extension support.
//!
//! Standard [`prost_types::MethodOptions`] drops the `google.api.http` extension
//! (field 72295728) during decoding because prost doesn't retain unknown fields.
//! These custom types preserve it.
//!
//! Used by both the build-time codegen and the `OpenAPI` spec generator.

#[allow(clippy::all, clippy::pedantic, clippy::nursery)]
mod types {
    use prost::Message;

    #[derive(Clone, PartialEq, Message)]
    pub struct FileDescriptorSet {
        #[prost(message, repeated, tag = "1")]
        pub file: Vec<FileDescriptorProto>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct FileDescriptorProto {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(string, optional, tag = "2")]
        pub package: Option<String>,
        #[prost(message, repeated, tag = "4")]
        pub message_type: Vec<DescriptorProto>,
        #[prost(message, repeated, tag = "5")]
        pub enum_type: Vec<EnumDescriptorProto>,
        #[prost(message, repeated, tag = "6")]
        pub service: Vec<ServiceDescriptorProto>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct DescriptorProto {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(message, repeated, tag = "2")]
        pub field: Vec<FieldDescriptorProto>,
        #[prost(message, repeated, tag = "3")]
        pub nested_type: Vec<DescriptorProto>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct FieldDescriptorProto {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        /// Protobuf field type enum: 1=double, 5=int32, 9=string, 11=message, 14=enum, …
        #[prost(int32, optional, tag = "5")]
        pub r#type: Option<i32>,
        /// Fully-qualified type name for message/enum fields (e.g., `.auth.v1.OAuthProvider`).
        #[prost(string, optional, tag = "6")]
        pub type_name: Option<String>,
        /// Field options including validation rules.
        #[prost(message, optional, tag = "8")]
        pub options: Option<FieldOptions>,
    }

    /// Field-level options, including `validate.rules` extension.
    #[derive(Clone, PartialEq, Message)]
    pub struct FieldOptions {
        /// `validate.rules` extension (tag 1071 from validate.proto).
        #[prost(message, optional, tag = "1071")]
        pub rules: Option<FieldRules>,
    }

    /// Minimal `validate.FieldRules` — only the rule types mapped to OpenAPI.
    #[derive(Clone, PartialEq, Message)]
    pub struct FieldRules {
        #[prost(message, optional, tag = "17")]
        pub message: Option<MessageRules>,
        #[prost(message, optional, tag = "3")]
        pub int32: Option<Int32Rules>,
        #[prost(message, optional, tag = "5")]
        pub uint32: Option<UInt32Rules>,
        #[prost(message, optional, tag = "6")]
        pub uint64: Option<UInt64Rules>,
        #[prost(message, optional, tag = "14")]
        pub string: Option<StringRules>,
        #[prost(message, optional, tag = "16")]
        pub r#enum: Option<EnumRules>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct MessageRules {
        #[prost(bool, optional, tag = "2")]
        pub required: Option<bool>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct StringRules {
        #[prost(uint64, optional, tag = "2")]
        pub min_len: Option<u64>,
        #[prost(uint64, optional, tag = "3")]
        pub max_len: Option<u64>,
        #[prost(string, optional, tag = "6")]
        pub pattern: Option<String>,
        #[prost(string, repeated, tag = "10")]
        pub r#in: Vec<String>,
        /// `well_known` oneof: `uuid = true` means the field must be a valid UUID.
        #[prost(bool, optional, tag = "22")]
        pub uuid: Option<bool>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Int32Rules {
        #[prost(int32, optional, tag = "2")]
        pub lt: Option<i32>,
        #[prost(int32, optional, tag = "3")]
        pub lte: Option<i32>,
        #[prost(int32, optional, tag = "4")]
        pub gt: Option<i32>,
        #[prost(int32, optional, tag = "5")]
        pub gte: Option<i32>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct UInt32Rules {
        #[prost(uint32, optional, tag = "2")]
        pub lt: Option<u32>,
        #[prost(uint32, optional, tag = "3")]
        pub lte: Option<u32>,
        #[prost(uint32, optional, tag = "4")]
        pub gt: Option<u32>,
        #[prost(uint32, optional, tag = "5")]
        pub gte: Option<u32>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct UInt64Rules {
        #[prost(uint64, optional, tag = "2")]
        pub lt: Option<u64>,
        #[prost(uint64, optional, tag = "3")]
        pub lte: Option<u64>,
        #[prost(uint64, optional, tag = "4")]
        pub gt: Option<u64>,
        #[prost(uint64, optional, tag = "5")]
        pub gte: Option<u64>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct EnumRules {
        #[prost(int32, repeated, tag = "4")]
        pub not_in: Vec<i32>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct EnumDescriptorProto {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(message, repeated, tag = "2")]
        pub value: Vec<EnumValueDescriptorProto>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct EnumValueDescriptorProto {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(int32, optional, tag = "2")]
        pub number: Option<i32>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct ServiceDescriptorProto {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(message, repeated, tag = "2")]
        pub method: Vec<MethodDescriptorProto>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct MethodDescriptorProto {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(string, optional, tag = "2")]
        pub input_type: Option<String>,
        #[prost(string, optional, tag = "3")]
        pub output_type: Option<String>,
        #[prost(message, optional, tag = "4")]
        pub options: Option<MethodOptions>,
        #[prost(bool, optional, tag = "5")]
        pub client_streaming: Option<bool>,
        #[prost(bool, optional, tag = "6")]
        pub server_streaming: Option<bool>,
    }

    /// Method options with the `google.api.http` extension (field 72295728).
    #[derive(Clone, PartialEq, Message)]
    pub struct MethodOptions {
        #[prost(message, optional, tag = "72295728")]
        pub http: Option<HttpRule>,
    }

    /// [`google.api.HttpRule`] — defines REST mapping for an RPC.
    #[derive(Clone, PartialEq, Message)]
    pub struct HttpRule {
        #[prost(oneof = "HttpPattern", tags = "2, 3, 4, 5, 6")]
        pub pattern: Option<HttpPattern>,
        #[prost(string, tag = "7")]
        pub body: String,
    }

    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum HttpPattern {
        #[prost(string, tag = "2")]
        Get(String),
        #[prost(string, tag = "3")]
        Put(String),
        #[prost(string, tag = "4")]
        Post(String),
        #[prost(string, tag = "5")]
        Delete(String),
        #[prost(string, tag = "6")]
        Patch(String),
    }
}

pub use types::*;

/// Proto field type constants (from `google.protobuf.FieldDescriptorProto.Type`).
pub mod field_type {
    /// `TYPE_INT32 = 5`
    pub const INT32: i32 = 5;
    /// `TYPE_INT64 = 3`
    pub const INT64: i32 = 3;
    /// `TYPE_UINT32 = 13`
    pub const UINT32: i32 = 13;
    /// `TYPE_UINT64 = 4`
    pub const UINT64: i32 = 4;
    /// `TYPE_BOOL = 8`
    pub const BOOL: i32 = 8;
    /// `TYPE_STRING = 9`
    pub const STRING: i32 = 9;
    /// `TYPE_MESSAGE = 11`
    pub const MESSAGE: i32 = 11;
    /// `TYPE_ENUM = 14`
    pub const ENUM: i32 = 14;
}

/// Extract `(http_method, path)` from a method's `google.api.http` annotation.
#[must_use]
pub fn extract_http_pattern(method: &MethodDescriptorProto) -> Option<(&'static str, &str)> {
    let pattern = method
        .options
        .as_ref()
        .and_then(|o| o.http.as_ref())
        .and_then(|h| h.pattern.as_ref())?;

    Some(match pattern {
        HttpPattern::Get(p) => ("get", p.as_str()),
        HttpPattern::Put(p) => ("put", p.as_str()),
        HttpPattern::Post(p) => ("post", p.as_str()),
        HttpPattern::Delete(p) => ("delete", p.as_str()),
        HttpPattern::Patch(p) => ("patch", p.as_str()),
    })
}

#[cfg(test)]
mod tests {
    use prost::Message as _;

    use super::*;

    fn method_with_pattern(pattern: HttpPattern) -> MethodDescriptorProto {
        MethodDescriptorProto {
            name: Some("TestMethod".to_string()),
            input_type: Some(".test.v1.Request".to_string()),
            output_type: Some(".test.v1.Response".to_string()),
            options: Some(MethodOptions {
                http: Some(HttpRule {
                    pattern: Some(pattern),
                    body: String::new(),
                }),
            }),
            client_streaming: None,
            server_streaming: None,
        }
    }

    #[test]
    fn extract_get_pattern() {
        let method = method_with_pattern(HttpPattern::Get("/v1/items".to_string()));
        let (http_method, path) = extract_http_pattern(&method).unwrap();
        assert_eq!(http_method, "get");
        assert_eq!(path, "/v1/items");
    }

    #[test]
    fn extract_post_pattern() {
        let method = method_with_pattern(HttpPattern::Post("/v1/items".to_string()));
        let (http_method, path) = extract_http_pattern(&method).unwrap();
        assert_eq!(http_method, "post");
        assert_eq!(path, "/v1/items");
    }

    #[test]
    fn extract_put_pattern() {
        let method = method_with_pattern(HttpPattern::Put("/v1/items/{id}".to_string()));
        let (http_method, path) = extract_http_pattern(&method).unwrap();
        assert_eq!(http_method, "put");
        assert_eq!(path, "/v1/items/{id}");
    }

    #[test]
    fn extract_delete_pattern() {
        let method = method_with_pattern(HttpPattern::Delete("/v1/items/{id}".to_string()));
        let (http_method, path) = extract_http_pattern(&method).unwrap();
        assert_eq!(http_method, "delete");
        assert_eq!(path, "/v1/items/{id}");
    }

    #[test]
    fn extract_patch_pattern() {
        let method = method_with_pattern(HttpPattern::Patch("/v1/items/{id}".to_string()));
        let (http_method, path) = extract_http_pattern(&method).unwrap();
        assert_eq!(http_method, "patch");
        assert_eq!(path, "/v1/items/{id}");
    }

    #[test]
    fn returns_none_without_options() {
        let method = MethodDescriptorProto {
            name: Some("NoOptions".to_string()),
            input_type: Some(".test.v1.Request".to_string()),
            output_type: Some(".test.v1.Response".to_string()),
            options: None,
            client_streaming: None,
            server_streaming: None,
        };
        assert!(extract_http_pattern(&method).is_none());
    }

    #[test]
    fn returns_none_without_http_rule() {
        let method = MethodDescriptorProto {
            name: Some("NoHttp".to_string()),
            input_type: Some(".test.v1.Request".to_string()),
            output_type: Some(".test.v1.Response".to_string()),
            options: Some(MethodOptions { http: None }),
            client_streaming: None,
            server_streaming: None,
        };
        assert!(extract_http_pattern(&method).is_none());
    }

    #[test]
    fn returns_none_without_pattern() {
        let method = MethodDescriptorProto {
            name: Some("NoPattern".to_string()),
            input_type: Some(".test.v1.Request".to_string()),
            output_type: Some(".test.v1.Response".to_string()),
            options: Some(MethodOptions {
                http: Some(HttpRule {
                    pattern: None,
                    body: "*".to_string(),
                }),
            }),
            client_streaming: None,
            server_streaming: None,
        };
        assert!(extract_http_pattern(&method).is_none());
    }

    #[test]
    fn field_type_constants() {
        assert_eq!(field_type::STRING, 9);
        assert_eq!(field_type::ENUM, 14);
    }

    /// Round-trip: encode → decode a `FileDescriptorSet` with HTTP annotations.
    #[test]
    fn descriptor_round_trip() {
        let original = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Req".to_string()),
                    field: vec![FieldDescriptorProto {
                        name: Some("name".to_string()),
                        r#type: Some(field_type::STRING),
                        type_name: None,
                        options: None,
                    }],
                    nested_type: vec![],
                }],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("Svc".to_string()),
                    method: vec![method_with_pattern(HttpPattern::Post(
                        "/v1/test".to_string(),
                    ))],
                }],
            }],
        };

        let bytes = original.encode_to_vec();
        let decoded = FileDescriptorSet::decode(bytes.as_slice()).unwrap();
        assert_eq!(original, decoded);
    }
}
