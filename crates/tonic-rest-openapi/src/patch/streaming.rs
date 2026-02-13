//! SSE streaming annotation transforms.
//!
//! Adds `x-streaming: sse` and `x-content-type: text/event-stream`
//! to server-streaming operations, and rewrites their response content type.

use serde_yaml_ng::Value;

use crate::discover::StreamingOp;

use super::helpers::{for_each_operation, val_s};

/// Annotate SSE streaming operations with custom extensions and correct content type.
///
/// Detection is method-aware: only the specific HTTP method that maps to a
/// server-streaming RPC gets annotated (e.g., `GET /v1/users` is streaming,
/// but `POST /v1/users` is not).
///
/// Falls back to a heuristic if the response schema `$ref` contains "stream".
pub fn annotate_sse(doc: &mut Value, streaming_ops: &[StreamingOp]) {
    for_each_operation(doc, |path, method, op_map| {
        let is_proto_streaming = streaming_ops
            .iter()
            .any(|op| op.method == method && op.path == path);

        if !is_proto_streaming && !is_streaming_heuristic(op_map) {
            return;
        }

        op_map.insert(val_s("x-streaming"), val_s("sse"));
        op_map.insert(val_s("x-content-type"), val_s("text/event-stream"));

        rewrite_sse_response_content_type(op_map);

        let existing = op_map
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("Server-sent events stream.")
            .to_string();

        if !existing.starts_with("**Streaming (SSE):**") {
            op_map.insert(
                val_s("description"),
                val_s(&format!("**Streaming (SSE):** {existing}")),
            );
        }

        // Add Last-Event-ID header parameter for SSE reconnection
        add_last_event_id_header(op_map);
    });
}

/// Add a `Last-Event-ID` header parameter for SSE reconnection.
fn add_last_event_id_header(op_map: &mut serde_yaml_ng::Mapping) {
    let params_key = val_s("parameters");
    if !op_map.contains_key(&params_key) {
        op_map.insert(params_key.clone(), Value::Sequence(Vec::new()));
    }

    let Some(params) = op_map.get_mut(&params_key).and_then(Value::as_sequence_mut) else {
        return;
    };

    // Don't add if already present
    let already_has = params.iter().any(|p| {
        p.as_mapping()
            .and_then(|m| m.get("name"))
            .and_then(Value::as_str)
            .is_some_and(|n| n == "Last-Event-ID")
    });

    if already_has {
        return;
    }

    let header: Value = serde_yaml_ng::from_str(
        r"
name: Last-Event-ID
in: header
required: false
description: >-
  Reconnection cursor from the last received SSE event.
  When set, the server resumes the stream from this point.
schema:
  type: string
",
    )
    .expect("static YAML must parse");

    params.push(header);
}

/// Check whether a response schema `$ref` contains "stream" (fallback heuristic).
fn is_streaming_heuristic(op: &serde_yaml_ng::Mapping) -> bool {
    op.get("responses")
        .and_then(Value::as_mapping)
        .and_then(|r| r.get("200"))
        .and_then(Value::as_mapping)
        .and_then(|r| r.get("content"))
        .and_then(Value::as_mapping)
        .and_then(|c| c.get("application/json"))
        .and_then(Value::as_mapping)
        .and_then(|mt| mt.get("schema"))
        .and_then(Value::as_mapping)
        .and_then(|s| s.get("$ref"))
        .and_then(Value::as_str)
        .is_some_and(|r| r.to_lowercase().contains("stream"))
}

/// Rewrite `200` response content type from `application/json` to `text/event-stream`.
fn rewrite_sse_response_content_type(op: &mut serde_yaml_ng::Mapping) {
    let Some(content) = op
        .get_mut("responses")
        .and_then(Value::as_mapping_mut)
        .and_then(|r| r.get_mut("200"))
        .and_then(Value::as_mapping_mut)
        .and_then(|r| r.get_mut("content"))
        .and_then(Value::as_mapping_mut)
    else {
        return;
    };

    let json_key = Value::String("application/json".to_string());
    if let Some(json_media_type) = content.remove(&json_key) {
        content.insert(
            Value::String("text/event-stream".to_string()),
            json_media_type,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotate_sse_marks_streaming_op() {
        let yaml = r"
paths:
  /v1/items:
    get:
      operationId: ItemService_ListItems
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Item'
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        let ops = vec![StreamingOp {
            method: "get".to_string(),
            path: "/v1/items".to_string(),
        }];

        annotate_sse(&mut doc, &ops);

        let op = doc["paths"]["/v1/items"]["get"].as_mapping().unwrap();
        assert_eq!(op.get("x-streaming").unwrap().as_str().unwrap(), "sse");
        assert_eq!(
            op.get("x-content-type").unwrap().as_str().unwrap(),
            "text/event-stream"
        );
        // Content type should be rewritten
        let content = op["responses"]["200"]["content"].as_mapping().unwrap();
        assert!(content.contains_key("text/event-stream"));
        assert!(!content.contains_key("application/json"));

        // Last-Event-ID header should be added
        let params = op.get("parameters").unwrap().as_sequence().unwrap();
        let last_event_id = params
            .iter()
            .find(|p| {
                p.as_mapping()
                    .and_then(|m| m.get("name"))
                    .and_then(Value::as_str)
                    .is_some_and(|n| n == "Last-Event-ID")
            })
            .expect("Last-Event-ID header should be added");
        assert_eq!(last_event_id["in"].as_str().unwrap(), "header");
        assert!(!last_event_id["required"].as_bool().unwrap());
    }

    #[test]
    fn annotate_sse_skips_non_streaming() {
        let yaml = r"
paths:
  /v1/items:
    post:
      operationId: ItemService_CreateItem
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Item'
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        annotate_sse(&mut doc, &[]);

        let op = doc["paths"]["/v1/items"]["post"].as_mapping().unwrap();
        assert!(!op.contains_key("x-streaming"));
    }
}
