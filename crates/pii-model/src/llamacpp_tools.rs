//! Helper functions for llama-cpp tool schema building and parsing.
//!
//! This module provides utilities for converting between Rust tool representations
//! and the JSON formats expected by llama-cpp models.

use crate::{ToolCall, ToolSpec};

/// Builds a JSON Schema for the model's function-call RESPONSE.
///
/// `json_schema_to_grammar` (llama.cpp) compiles a JSON Schema into a GBNF
/// grammar, so the schema must describe the shape the model should emit —
/// NOT the `OpenAI` tool-list wire format. We expect the model to emit a
/// single object `{"name": <one of the tool names>, "arguments": {...}}`,
/// and — critically — the `arguments` for a given `name` must be constrained
/// to THAT tool's own parameter schema. A bare `"arguments": {"type":"object"}`
/// lets the model invent key names (verified: it emitted
/// `{"location":"Paris","method":"get_weather"}` where the tool required
/// `city`), producing useless calls.
///
/// The fix is a `oneOf` over the tools, each binding a `const` name to that
/// tool's real parameters schema:
///
/// ```json
/// {
///   "oneOf": [
///     {
///       "type": "object",
///       "properties": {
///         "name": { "const": "get_weather" },
///         "arguments": { <get_weather's schema> }
///       },
///       "required": ["name", "arguments"],
///       "additionalProperties": false
///     }
///   ]
/// }
/// ```
#[must_use]
pub fn build_schema_from_tools(tools: &[ToolSpec]) -> String {
    let variants: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "const": t.name },
                    "arguments": t.schema
                },
                "required": ["name", "arguments"],
                "additionalProperties": false
            })
        })
        .collect();
    let schema = serde_json::json!({ "oneOf": variants });
    serde_json::to_string(&schema).unwrap_or_default()
}

/// Parses model output into a vector of `ToolCall` objects.
///
/// Accepts BOTH shapes:
/// - `{"name":"x","arguments":{...}}`
/// - `{"function":{"name":"x","arguments":{...}}}`
///
/// For `arguments`, if the JSON value is a string, it is parsed as JSON;
/// otherwise it is used as-is. On any parse failure, returns an empty vec.
///
/// # Arguments
/// * `output` - The model's JSON output string.
///
/// # Returns
/// A vector of `ToolCall` objects, or an empty vec on parse failure.
pub fn parse_tool_calls(output: &str) -> Vec<ToolCall> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
        return Vec::new();
    };

    // Handle both single object and array of objects
    let items: Vec<serde_json::Value> = if let Some(arr) = value.as_array() {
        arr.clone()
    } else if value.is_object() {
        vec![value]
    } else {
        return Vec::new();
    };

    let mut result = Vec::new();

    for item in items {
        // Try wrapper shape first: {"function":{"name":"x","arguments":{...}}}
        if let Some(func_obj) = item.get("function").and_then(|f| f.as_object()) {
            if let (Some(name), Some(arguments)) = (
                func_obj.get("name").and_then(|n| n.as_str()),
                func_obj.get("arguments"),
            ) {
                let args = arguments.as_str().map_or_else(
                    || arguments.clone(),
                    |args_str| serde_json::from_str(args_str).unwrap_or_else(|_| arguments.clone()),
                );

                result.push(ToolCall {
                    id: String::new(),
                    name: name.to_string(),
                    arguments: args,
                });
            }
        }
        // Try plain shape: {"name":"x","arguments":{...}}
        else if let (Some(name), Some(arguments)) = (
            item.get("name").and_then(|n| n.as_str()),
            item.get("arguments"),
        ) {
            let args = arguments.as_str().map_or_else(
                || arguments.clone(),
                |args_str| serde_json::from_str(args_str).unwrap_or_else(|_| arguments.clone()),
            );

            result.push(ToolCall {
                id: String::new(),
                name: name.to_string(),
                arguments: args,
            });
        }
    }

    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn schema_includes_tool_names() {
        let tools = vec![
            ToolSpec {
                name: "fs_read".to_string(),
                description: "read".to_string(),
                schema: serde_json::json!({ "type": "object" }),
            },
            ToolSpec {
                name: "fs_write".to_string(),
                description: "write".to_string(),
                schema: serde_json::json!({ "type": "object" }),
            },
        ];

        let result = build_schema_from_tools(&tools);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("schema string must parse to JSON");

        let variants = parsed
            .get("oneOf")
            .and_then(|v| v.as_array())
            .expect("schema must have a oneOf array");
        assert_eq!(variants.len(), 2, "one variant per tool");
        let names: Vec<&str> = variants
            .iter()
            .filter_map(|v| v.get("properties")?.get("name")?.get("const")?.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["fs_read", "fs_write"],
            "each variant must bind a const tool name: {parsed}"
        );
    }

    #[test]
    fn parse_plain_shape() {
        let input = r#"{"name":"fs_read","arguments":{"path":"/x"}}"#;
        let result = parse_tool_calls(input);

        assert_eq!(result.len(), 1, "should parse one tool call");
        assert_eq!(result.first().expect("one tool call").name, "fs_read");
        assert!(
            result
                .first()
                .expect("one tool call")
                .arguments
                .get("path")
                .is_some()
        );
    }

    #[test]
    fn parse_wrapper_shape() {
        let input = r#"{"function":{"name":"fs_read","arguments":{"path":"/x"}}}"#;
        let result = parse_tool_calls(input);

        assert_eq!(result.len(), 1, "should parse one tool call");
        assert_eq!(result.first().expect("one tool call").name, "fs_read");
    }

    #[test]
    fn parse_invalid_returns_empty() {
        let input = r#""not json""#;
        let result = parse_tool_calls(input);

        assert!(result.is_empty(), "invalid JSON should return empty vec");
    }
}
