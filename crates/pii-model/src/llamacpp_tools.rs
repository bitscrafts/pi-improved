//! Helper functions for llama-cpp tool schema building and parsing.
//!
//! This module provides utilities for converting between Rust tool representations
//! and the JSON formats expected by llama-cpp models.

use crate::{ToolCall, ToolSpec};

/// Builds a JSON schema object from a list of tool specifications.
///
/// Each tool contributes its `name`, `description`, and `parameters` (the tool's
/// `schema` field) into a JSON object. The returned string is the JSON
/// serialization of the complete object.
///
/// # Arguments
/// * `tools` - Slice of `ToolSpec` to include in the schema.
///
/// # Returns
/// A JSON string representing the tool definitions object.
pub fn build_schema_from_tools(tools: &[ToolSpec]) -> String {
    let mut schema = serde_json::Map::new();

    for tool in tools {
        let mut tool_obj = serde_json::Map::new();
        tool_obj.insert("type".to_string(), serde_json::json!("function"));
        tool_obj.insert(
            "function".to_string(),
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.schema
            }),
        );

        schema.insert(tool.name.clone(), serde_json::Value::Object(tool_obj));
    }

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
    use super::*;

    #[test]
    fn schema_includes_tool_names() {
        let tools = vec![ToolSpec {
            name: "fs_read".to_string(),
            description: "r".to_string(),
            schema: serde_json::json!({ "type": "object" }),
        }];

        let result = build_schema_from_tools(&tools);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert!(parsed.is_object());
        assert!(
            parsed.get("fs_read").is_some(),
            "schema should contain 'fs_read'"
        );
    }

    #[test]
    fn parse_plain_shape() {
        let input = r#"{"name":"fs_read","arguments":{"path":"/x"}}"#;
        let result = parse_tool_calls(input);

        assert_eq!(result.len(), 1, "should parse one tool call");
        assert_eq!(result[0].name, "fs_read");
        assert!(result[0].arguments.get("path").is_some());
    }

    #[test]
    fn parse_wrapper_shape() {
        let input = r#"{"function":{"name":"fs_read","arguments":{"path":"/x"}}}"#;
        let result = parse_tool_calls(input);

        assert_eq!(result.len(), 1, "should parse one tool call");
        assert_eq!(result[0].name, "fs_read");
    }

    #[test]
    fn parse_invalid_returns_empty() {
        let input = r#""not json""#;
        let result = parse_tool_calls(input);

        assert!(result.is_empty(), "invalid JSON should return empty vec");
    }
}
