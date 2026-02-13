//! Before/after fixture tests for the full patch pipeline.
//!
//! Each test provides a minimal input YAML and verifies the output
//! after applying [`tonic_rest_openapi::patch`] with specific config.

use pretty_assertions::assert_eq;
use serde_yaml_ng::Value;

use tonic_rest_openapi::internal::{
    EnumRewrite, FieldConstraint, OperationEntry, SchemaConstraints, StreamingOp,
};
use tonic_rest_openapi::{PatchConfig, ProtoMetadata};

/// Build minimal valid metadata with defaults.
fn empty_metadata() -> ProtoMetadata {
    ProtoMetadata::default()
}

/// Helper to parse YAML, run the patch pipeline, and return the result value.
fn run_patch(input: &str, config: &PatchConfig<'_>) -> Value {
    let output = tonic_rest_openapi::patch(input, config).expect("patch should succeed");
    serde_yaml_ng::from_str(&output).expect("output should parse")
}

#[test]
fn oas31_upgrade_full_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
components:
  schemas:
    Foo:
      type: object
      properties:
        bar:
          type: string
          nullable: true
        baz:
          type: integer
          nullable: false
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    // Version upgraded
    assert_eq!(result["openapi"].as_str().unwrap(), "3.1.0");

    // nullable: true → type: [string, "null"]
    let bar = &result["components"]["schemas"]["Foo"]["properties"]["bar"];
    let types = bar["type"].as_sequence().unwrap();
    assert!(types.contains(&Value::String("string".to_string())));
    assert!(types.contains(&Value::String("null".to_string())));

    // nullable: false → removed, type stays scalar
    let baz = &result["components"]["schemas"]["Foo"]["properties"]["baz"];
    assert_eq!(baz["type"].as_str().unwrap(), "integer");
    assert!(baz.as_mapping().unwrap().get("nullable").is_none());
}

#[test]
fn streaming_annotation_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths:
  /v1/users:
    get:
      operationId: UserService_ListUsers
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                type: string
";

    let mut metadata = empty_metadata();
    metadata.set_streaming_ops(vec![StreamingOp {
        method: "get".to_string(),
        path: "/v1/users".to_string(),
    }]);

    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    let op = &result["paths"]["/v1/users"]["get"];
    assert_eq!(op["x-streaming"].as_str().unwrap(), "sse");

    // Content type changed from application/json to text/event-stream
    let content = op["responses"]["200"]["content"].as_mapping().unwrap();
    assert!(content.contains_key("text/event-stream"));
    assert!(!content.contains_key("application/json"));
}

#[test]
fn validation_constraints_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
components:
  schemas:
    test.v1.SignUpRequest:
      type: object
      properties:
        email:
          type: string
        password:
          type: string
";

    let mut metadata = empty_metadata();
    metadata.set_field_constraints(vec![SchemaConstraints {
        schema: "test.v1.SignUpRequest".to_string(),
        fields: vec![
            FieldConstraint {
                field: "email".to_string(),
                min: Some(5),
                max: Some(255),
                signed_min: None,
                signed_max: None,
                pattern: Some(r"^[^@\s]+@[^@\s]+$".to_string()),
                enum_values: Vec::new(),
                required: true,
                is_uuid: false,
                is_numeric: false,
            },
            FieldConstraint {
                field: "password".to_string(),
                min: Some(8),
                max: Some(128),
                signed_min: None,
                signed_max: None,
                pattern: None,
                enum_values: Vec::new(),
                required: true,
                is_uuid: false,
                is_numeric: false,
            },
        ],
    }]);

    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    let schema = &result["components"]["schemas"]["test.v1.SignUpRequest"];
    let email = &schema["properties"]["email"];
    assert_eq!(email["minLength"].as_u64().unwrap(), 5);
    assert_eq!(email["maxLength"].as_u64().unwrap(), 255);
    assert!(email["pattern"].as_str().is_some());

    let password = &schema["properties"]["password"];
    assert_eq!(password["minLength"].as_u64().unwrap(), 8);
    assert_eq!(password["maxLength"].as_u64().unwrap(), 128);

    // Required array
    let required = schema["required"].as_sequence().unwrap();
    assert!(required.contains(&Value::String("email".to_string())));
    assert!(required.contains(&Value::String("password".to_string())));
}

#[test]
fn security_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths:
  /v1/auth:
    post:
      operationId: AuthService_Authenticate
      responses:
        '200':
          description: OK
  /v1/sessions:
    get:
      operationId: AuthService_ListSessions
      responses:
        '200':
          description: OK
components:
  schemas: {}
";

    let mut metadata = empty_metadata();
    metadata.set_operation_ids(vec![
        OperationEntry {
            method_name: "Authenticate".to_string(),
            operation_id: "AuthService_Authenticate".to_string(),
        },
        OperationEntry {
            method_name: "ListSessions".to_string(),
            operation_id: "AuthService_ListSessions".to_string(),
        },
    ]);

    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false)
        .public_methods(&["Authenticate"]);

    let result = run_patch(input, &config);

    // Global security requires bearerAuth
    let security = result["security"].as_sequence().unwrap();
    assert!(!security.is_empty());

    // securitySchemes defined
    let schemes = result["components"]["securitySchemes"]
        .as_mapping()
        .unwrap();
    assert!(schemes.contains_key("bearerAuth"));

    // Public endpoint has empty security
    let auth_op = &result["paths"]["/v1/auth"]["post"];
    let auth_sec = auth_op["security"].as_sequence().unwrap();
    assert!(auth_sec.is_empty());

    // Protected endpoint has no per-operation override (inherits global)
    let sessions_op = &result["paths"]["/v1/sessions"]["get"];
    assert!(sessions_op.as_mapping().unwrap().get("security").is_none());
}

#[test]
fn enum_rewrite_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths:
  /v1/users:
    get:
      operationId: UserService_ListUsers
      parameters:
        - name: status
          in: query
          schema:
            type: string
            enum:
              - USER_STATUS_UNSPECIFIED
              - USER_STATUS_ACTIVE
              - USER_STATUS_SUSPENDED
      responses:
        '200':
          description: OK
components:
  schemas:
    users.v1.User:
      type: object
      properties:
        status:
          type: string
          format: enum
          enum:
            - USER_STATUS_UNSPECIFIED
            - USER_STATUS_ACTIVE
            - USER_STATUS_SUSPENDED
";

    let mut metadata = empty_metadata();
    metadata.set_enum_rewrites(vec![EnumRewrite {
        schema: "users.v1.User".to_string(),
        field: "status".to_string(),
        values: vec![
            "unspecified".to_string(),
            "active".to_string(),
            "suspended".to_string(),
        ],
    }]);
    metadata.set_enum_value_map(
        [
            (
                "USER_STATUS_UNSPECIFIED".to_string(),
                "unspecified".to_string(),
            ),
            ("USER_STATUS_ACTIVE".to_string(), "active".to_string()),
            ("USER_STATUS_SUSPENDED".to_string(), "suspended".to_string()),
        ]
        .into(),
    );

    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    // Component schema enum should be rewritten
    let schema_enum = result["components"]["schemas"]["users.v1.User"]["properties"]["status"]
        ["enum"]
        .as_sequence()
        .unwrap();
    assert!(schema_enum.contains(&Value::String("active".to_string())));
    assert!(!schema_enum
        .iter()
        .any(|v| v.as_str().unwrap().contains("USER_STATUS")));

    // format: enum should be removed
    let props = result["components"]["schemas"]["users.v1.User"]["properties"]["status"]
        .as_mapping()
        .unwrap();
    assert!(!props.contains_key("format"));

    // Query param enum should have UNSPECIFIED stripped and values rewritten
    let param_enum = result["paths"]["/v1/users"]["get"]["parameters"][0]["schema"]["enum"]
        .as_sequence()
        .unwrap();
    assert!(!param_enum.iter().any(|v| {
        v.as_str()
            .is_some_and(|s| s.contains("UNSPECIFIED") || s.contains("unspecified"))
    }));
}

#[test]
fn request_body_inlining_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths:
  /v1/auth:
    post:
      operationId: AuthService_Authenticate
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/auth.v1.AuthRequest'
      responses:
        '200':
          description: OK
components:
  schemas:
    auth.v1.AuthRequest:
      type: object
      description: Authentication credentials.
      properties:
        email:
          type: string
        password:
          type: string
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    let media = &result["paths"]["/v1/auth"]["post"]["requestBody"]["content"]["application/json"];

    // Schema should be inlined (no $ref)
    let schema = media["schema"].as_mapping().unwrap();
    assert!(schema.contains_key("properties"));
    assert!(!schema.contains_key("$ref"));

    // Example should be generated
    let example = media["example"].as_mapping().unwrap();
    assert!(example.contains_key("email"));
    assert!(example.contains_key("password"));

    // Description moved to requestBody level
    let rb = result["paths"]["/v1/auth"]["post"]["requestBody"]
        .as_mapping()
        .unwrap();
    assert_eq!(
        rb.get("description").unwrap().as_str().unwrap(),
        "Authentication credentials."
    );

    // Original schema should be removed (orphaned)
    let schemas = result["components"]["schemas"].as_mapping().unwrap();
    assert!(!schemas.contains_key("auth.v1.AuthRequest"));
}

// --- Error path tests ---

#[test]
fn patch_rejects_invalid_yaml() {
    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata);

    let result = tonic_rest_openapi::patch("[[[invalid yaml", &config);
    assert!(result.is_err(), "invalid YAML should produce an error");
}

#[test]
fn patch_rejects_unresolvable_unimplemented_method() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata).unimplemented_methods(&["NonExistentMethod"]);

    let result = tonic_rest_openapi::patch(input, &config);
    assert!(
        result.is_err(),
        "unresolvable method name should produce an error"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("NonExistentMethod"),
        "error should mention the method name: {err}",
    );
}

#[test]
fn patch_rejects_unresolvable_public_method() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata).public_methods(&["GhostMethod"]);

    let result = tonic_rest_openapi::patch(input, &config);
    assert!(
        result.is_err(),
        "unresolvable public method should produce an error"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("GhostMethod"),
        "error should mention the method name: {err}",
    );
}

#[test]
fn patch_rejects_ambiguous_method_name() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
";

    let mut metadata = empty_metadata();
    metadata.set_operation_ids(vec![
        OperationEntry {
            method_name: "Delete".to_string(),
            operation_id: "AuthService_Delete".to_string(),
        },
        OperationEntry {
            method_name: "Delete".to_string(),
            operation_id: "UserService_Delete".to_string(),
        },
    ]);

    let config = PatchConfig::new(&metadata).unimplemented_methods(&["Delete"]);

    let result = tonic_rest_openapi::patch(input, &config);
    assert!(result.is_err(), "ambiguous method should produce an error");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("ambiguous"),
        "error should mention ambiguity: {err}",
    );
    assert!(
        err.contains("Service.Method"),
        "error should suggest qualification: {err}",
    );
}
