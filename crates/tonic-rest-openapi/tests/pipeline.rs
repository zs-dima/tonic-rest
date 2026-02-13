//! Before/after fixture tests for the full patch pipeline.
//!
//! Each test provides a minimal input YAML and verifies the output
//! after applying [`tonic_rest_openapi::patch`] with specific config.

use pretty_assertions::assert_eq;
use serde_yaml_ng::Value;

use tonic_rest_openapi::{
    ContactInfo, EnumRewrite, ExternalDocsInfo, FieldConstraint, InfoOverrides, LicenseInfo,
    OperationEntry, PatchConfig, ProtoMetadata, SchemaConstraints, ServerEntry, StreamingOp,
};

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
    let schema_enum =
        result["components"]["schemas"]["users.v1.User"]["properties"]["status"]["enum"]
            .as_sequence()
            .unwrap();
    assert!(schema_enum.contains(&Value::String("active".to_string())));
    assert!(
        !schema_enum
            .iter()
            .any(|v| v.as_str().unwrap().contains("USER_STATUS"))
    );

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

    // Examples should be on individual properties, not media-type level
    assert!(
        media.as_mapping().unwrap().get("example").is_none(),
        "media-type-level example should not be present"
    );
    let email_prop = schema
        .get("properties")
        .unwrap()
        .as_mapping()
        .unwrap()
        .get("email")
        .unwrap()
        .as_mapping()
        .unwrap();
    assert!(
        email_prop.contains_key("example"),
        "email property should have inline example"
    );
    let password_prop = schema
        .get("properties")
        .unwrap()
        .as_mapping()
        .unwrap()
        .get("password")
        .unwrap()
        .as_mapping()
        .unwrap();
    assert!(
        password_prop.contains_key("example"),
        "password property should have inline example"
    );

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

#[test]
fn servers_injection_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    // Default server entry should be injected
    let servers = result["servers"].as_sequence().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["url"].as_str().unwrap(), "http://localhost:8080");
    assert_eq!(
        servers[0]["description"].as_str().unwrap(),
        "Local development"
    );
}

#[test]
fn servers_custom_entries_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false)
        .servers(&[
            ServerEntry {
                url: "https://api.example.com".to_string(),
                description: Some("Production".to_string()),
            },
            ServerEntry {
                url: "https://staging.example.com".to_string(),
                description: None,
            },
        ]);

    let result = run_patch(input, &config);

    let servers = result["servers"].as_sequence().unwrap();
    assert_eq!(servers.len(), 2);
    assert_eq!(
        servers[0]["url"].as_str().unwrap(),
        "https://api.example.com"
    );
    assert_eq!(servers[0]["description"].as_str().unwrap(), "Production");
    assert_eq!(
        servers[1]["url"].as_str().unwrap(),
        "https://staging.example.com"
    );
}

#[test]
fn info_enrichment_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false)
        .info(InfoOverrides {
            contact: Some(ContactInfo {
                name: Some("API Support".to_string()),
                email: Some("support@example.com".to_string()),
                url: None,
            }),
            license: Some(LicenseInfo {
                name: "MIT".to_string(),
                url: Some("https://opensource.org/licenses/MIT".to_string()),
            }),
            external_docs: Some(ExternalDocsInfo {
                url: "https://docs.example.com".to_string(),
                description: Some("Full documentation".to_string()),
            }),
            terms_of_service: Some("https://example.com/tos".to_string()),
        });

    let result = run_patch(input, &config);

    let info = &result["info"];
    assert_eq!(info["contact"]["name"].as_str().unwrap(), "API Support");
    assert_eq!(
        info["contact"]["email"].as_str().unwrap(),
        "support@example.com"
    );
    assert_eq!(info["license"]["name"].as_str().unwrap(), "MIT");
    assert_eq!(
        info["termsOfService"].as_str().unwrap(),
        "https://example.com/tos"
    );

    let ext = &result["externalDocs"];
    assert_eq!(ext["url"].as_str().unwrap(), "https://docs.example.com");
    assert_eq!(ext["description"].as_str().unwrap(), "Full documentation");
}

#[test]
fn uuid_path_template_flattened_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths:
  /v1/users/{userId.value}:
    get:
      operationId: UserService_GetUser
      parameters:
        - name: userId.value
          in: path
          required: true
          schema:
            type: string
      responses:
        '200':
          description: OK
components:
  schemas: {}
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    // Path key should be flattened
    let paths = result["paths"].as_mapping().unwrap();
    assert!(paths.contains_key("/v1/users/{userId}"));
    assert!(!paths.contains_key("/v1/users/{userId.value}"));

    // Parameter name should be flattened
    let param = &result["paths"]["/v1/users/{userId}"]["get"]["parameters"][0];
    assert_eq!(param["name"].as_str().unwrap(), "userId");
}

#[test]
fn deprecated_operations_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths:
  /v1/legacy:
    get:
      operationId: LegacyService_GetOldData
      responses:
        '200':
          description: OK
  /v1/current:
    get:
      operationId: CurrentService_GetData
      responses:
        '200':
          description: OK
components:
  schemas: {}
";

    let mut metadata = empty_metadata();
    metadata.set_operation_ids(vec![
        OperationEntry {
            method_name: "GetOldData".to_string(),
            operation_id: "LegacyService_GetOldData".to_string(),
        },
        OperationEntry {
            method_name: "GetData".to_string(),
            operation_id: "CurrentService_GetData".to_string(),
        },
    ]);

    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false)
        .deprecated_methods(&["GetOldData"]);

    let result = run_patch(input, &config);

    // Deprecated op should have deprecated: true
    let legacy_op = &result["paths"]["/v1/legacy"]["get"];
    assert_eq!(legacy_op["deprecated"].as_bool().unwrap(), true);

    // Non-deprecated op should not have deprecated flag
    let current_op = &result["paths"]["/v1/current"]["get"];
    assert!(current_op.as_mapping().unwrap().get("deprecated").is_none());
}

#[test]
fn create_response_rewrite_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths:
  /v1/users:
    post:
      operationId: UserService_CreateUser
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                type: object
  /v1/auth/signup:
    post:
      operationId: AuthService_SignUp
      responses:
        '200':
          description: OK
  /v1/auth/login:
    post:
      operationId: AuthService_Login
      responses:
        '200':
          description: OK
components:
  schemas: {}
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    // CreateUser → 201 Created
    let create_responses = result["paths"]["/v1/users"]["post"]["responses"]
        .as_mapping()
        .unwrap();
    assert!(create_responses.contains_key("201"));
    assert!(!create_responses.contains_key("200"));
    assert_eq!(
        result["paths"]["/v1/users"]["post"]["responses"]["201"]["description"]
            .as_str()
            .unwrap(),
        "Created"
    );

    // SignUp → 201 Created
    let signup_responses = result["paths"]["/v1/auth/signup"]["post"]["responses"]
        .as_mapping()
        .unwrap();
    assert!(signup_responses.contains_key("201"));
    assert!(!signup_responses.contains_key("200"));

    // Login → stays 200 (not a Create/SignUp/Register)
    let login_responses = result["paths"]["/v1/auth/login"]["post"]["responses"]
        .as_mapping()
        .unwrap();
    assert!(login_responses.contains_key("200"));
    assert!(!login_responses.contains_key("201"));
}

#[test]
fn unspecified_stripped_from_schemas_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
components:
  schemas:
    users.v1.User:
      type: object
      properties:
        role:
          type: string
          enum:
            - ROLE_UNSPECIFIED
            - ROLE_ADMIN
            - ROLE_USER
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    let role_enum = result["components"]["schemas"]["users.v1.User"]["properties"]["role"]["enum"]
        .as_sequence()
        .unwrap();
    assert!(
        !role_enum
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains("UNSPECIFIED")))
    );
    assert!(role_enum.contains(&Value::String("ROLE_ADMIN".to_string())));
    assert!(role_enum.contains(&Value::String("ROLE_USER".to_string())));
}

#[test]
fn field_access_annotation_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
components:
  schemas:
    users.v1.User:
      type: object
      properties:
        password:
          type: string
        clientSecret:
          type: string
        createdAt:
          type: string
        updatedAt:
          type: string
        displayName:
          type: string
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    let props = &result["components"]["schemas"]["users.v1.User"]["properties"];

    // Convention-based writeOnly
    assert_eq!(props["password"]["writeOnly"].as_bool().unwrap(), true);
    assert_eq!(props["clientSecret"]["writeOnly"].as_bool().unwrap(), true);

    // Convention-based readOnly
    assert_eq!(props["createdAt"]["readOnly"].as_bool().unwrap(), true);
    assert_eq!(props["updatedAt"]["readOnly"].as_bool().unwrap(), true);

    // displayName should have neither
    let display = props["displayName"].as_mapping().unwrap();
    assert!(display.get("readOnly").is_none());
    assert!(display.get("writeOnly").is_none());
}

#[test]
fn field_access_extra_patterns_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
components:
  schemas:
    test.v1.Config:
      type: object
      properties:
        api_token:
          type: string
        revision:
          type: integer
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false)
        .write_only_fields(&["api_token"])
        .read_only_fields(&["revision"]);

    let result = run_patch(input, &config);

    let props = &result["components"]["schemas"]["test.v1.Config"]["properties"];
    assert_eq!(props["api_token"]["writeOnly"].as_bool().unwrap(), true);
    assert_eq!(props["revision"]["readOnly"].as_bool().unwrap(), true);
}

#[test]
fn duration_fields_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths: {}
components:
  schemas:
    google.protobuf.Duration:
      type: object
      properties:
        seconds:
          type: string
        nanos:
          type: integer
    test.v1.Session:
      type: object
      properties:
        timeout:
          $ref: '#/components/schemas/google.protobuf.Duration'
";

    let metadata = empty_metadata();
    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .annotate_sse(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    // Duration schema should be replaced with string type
    let dur = &result["components"]["schemas"]["google.protobuf.Duration"];
    assert_eq!(dur["type"].as_str().unwrap(), "string");
    assert_eq!(dur["example"].as_str().unwrap(), "300s");

    // Reference should be kept intact (just pointing to the rewritten schema)
    let timeout = &result["components"]["schemas"]["test.v1.Session"]["properties"]["timeout"];
    assert_eq!(
        timeout["$ref"].as_str().unwrap(),
        "#/components/schemas/google.protobuf.Duration"
    );
}

#[test]
fn last_event_id_header_pipeline() {
    let input = r"
openapi: 3.0.3
info:
  title: Test
  version: 0.1.0
paths:
  /v1/events:
    get:
      operationId: EventService_WatchEvents
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
        path: "/v1/events".to_string(),
    }]);

    let config = PatchConfig::new(&metadata)
        .upgrade_to_3_1(false)
        .inject_validation(false)
        .add_security(false)
        .inline_request_bodies(false)
        .flatten_uuid_refs(false);

    let result = run_patch(input, &config);

    // Should have Last-Event-ID header parameter
    let params = result["paths"]["/v1/events"]["get"]["parameters"]
        .as_sequence()
        .unwrap();
    let last_event_id = params.iter().find(|p| {
        p["name"].as_str() == Some("Last-Event-ID") && p["in"].as_str() == Some("header")
    });
    assert!(
        last_event_id.is_some(),
        "Last-Event-ID header parameter should be present"
    );
    let param = last_event_id.unwrap();
    assert_eq!(param["required"].as_bool().unwrap(), false);
    assert_eq!(param["schema"]["type"].as_str().unwrap(), "string");
}
