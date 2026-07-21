use serde_json::{Map, Value};

use crate::McpError;

pub const MAX_MCP_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_SCHEMA_DEPTH: usize = 32;
const MAX_SCHEMA_NODES: usize = 4_096;
const MAX_SCHEMA_PROPERTIES: usize = 256;
const MAX_SCHEMA_STRING_BYTES: usize = 16 * 1024;

/// Validates a server-provided input schema before it can become a tool contract.
///
/// # Errors
///
/// Returns an error when the schema is malformed, unsafe to resolve, or exceeds
/// Pactrail's structural or serialized bounds.
pub fn validate_input_schema(tool: &str, schema: &Value) -> Result<(), McpError> {
    validate_schema(tool, schema, "input")
}

/// Validates a server-provided structured output schema under the same trust bounds.
///
/// # Errors
///
/// Returns an error when the schema is malformed, unsafe to resolve, or exceeds
/// Pactrail's structural or serialized bounds.
pub fn validate_output_schema(tool: &str, schema: &Value) -> Result<(), McpError> {
    validate_schema(tool, schema, "output")
}

fn validate_schema(tool: &str, schema: &Value, kind: &str) -> Result<(), McpError> {
    let encoded = serde_json::to_vec(schema)?;
    if encoded.len() > MAX_MCP_SCHEMA_BYTES {
        return Err(invalid(
            tool,
            format!("serialized schema exceeds {MAX_MCP_SCHEMA_BYTES} bytes"),
        ));
    }
    let object = schema
        .as_object()
        .ok_or_else(|| invalid(tool, format!("{kind} schema root must be an object")))?;
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err(invalid(
            tool,
            format!("{kind} schema root type must be object"),
        ));
    }
    let mut nodes = 0_usize;
    inspect_node(tool, schema, 0, &mut nodes)?;
    jsonschema::validator_for(schema)
        .map_err(|error| invalid(tool, format!("JSON Schema compilation failed: {error}")))?;
    Ok(())
}

/// Validates one tool call locally against its pinned schema.
///
/// # Errors
///
/// Returns an error when arguments are not an object or do not match the pinned schema.
pub fn validate_arguments(tool: &str, schema: &Value, arguments: &Value) -> Result<(), McpError> {
    let Some(_arguments) = arguments.as_object() else {
        return Err(invalid(tool, "tool arguments must be a JSON object"));
    };
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| invalid(tool, format!("pinned schema no longer compiles: {error}")))?;
    if let Err(error) = validator.validate(arguments) {
        let mut diagnostic = error.to_string();
        if diagnostic.len() > 1_024 {
            diagnostic.truncate(1_024);
            diagnostic.push_str("...");
        }
        return Err(invalid(
            tool,
            format!("arguments do not match schema: {diagnostic}"),
        ));
    }
    Ok(())
}

fn inspect_node(
    tool: &str,
    value: &Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), McpError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(invalid(
            tool,
            format!("schema exceeds maximum depth {MAX_SCHEMA_DEPTH}"),
        ));
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or_else(|| invalid(tool, "schema node count overflowed"))?;
    if *nodes > MAX_SCHEMA_NODES {
        return Err(invalid(
            tool,
            format!("schema exceeds {MAX_SCHEMA_NODES} nodes"),
        ));
    }
    match value {
        Value::Object(object) => inspect_object(tool, object, depth, nodes),
        Value::Array(items) => {
            for item in items {
                inspect_node(tool, item, depth + 1, nodes)?;
            }
            Ok(())
        }
        Value::String(text) if text.len() > MAX_SCHEMA_STRING_BYTES => Err(invalid(
            tool,
            format!("schema string exceeds {MAX_SCHEMA_STRING_BYTES} bytes"),
        )),
        _ => Ok(()),
    }
}

fn inspect_object(
    tool: &str,
    object: &Map<String, Value>,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), McpError> {
    if object.contains_key("$ref") || object.contains_key("$dynamicRef") {
        return Err(invalid(
            tool,
            "schema references are disabled at the MCP trust boundary",
        ));
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object)
        && properties.len() > MAX_SCHEMA_PROPERTIES
    {
        return Err(invalid(
            tool,
            format!("schema object exceeds {MAX_SCHEMA_PROPERTIES} properties"),
        ));
    }
    for (key, value) in object {
        if key.len() > MAX_SCHEMA_STRING_BYTES {
            return Err(invalid(
                tool,
                format!("schema key exceeds {MAX_SCHEMA_STRING_BYTES} bytes"),
            ));
        }
        inspect_node(tool, value, depth + 1, nodes)?;
    }
    Ok(())
}

fn invalid(tool: &str, reason: impl Into<String>) -> McpError {
    McpError::InvalidSchema {
        tool: tool.to_owned(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::{validate_arguments, validate_input_schema};

    #[test]
    fn valid_object_schema_checks_arguments() {
        let schema = json!({
            "type": "object",
            "properties": { "count": { "type": "integer", "minimum": 1 } },
            "required": ["count"],
            "additionalProperties": false
        });
        assert!(validate_input_schema("counter", &schema).is_ok());
        assert!(validate_arguments("counter", &schema, &json!({"count": 2})).is_ok());
        assert!(validate_arguments("counter", &schema, &json!({"count": 0})).is_err());
        assert!(validate_arguments("counter", &schema, &json!({"count": 2, "x": 1})).is_err());
    }

    #[test]
    fn schema_references_and_deep_trees_fail_closed() {
        let referenced = json!({
            "type": "object",
            "properties": { "value": { "$ref": "https://attacker.invalid/schema" } }
        });
        assert!(validate_input_schema("referenced", &referenced).is_err());

        let mut deep = json!({"type": "object"});
        for _ in 0..40 {
            deep = json!({"type": "object", "properties": {"next": deep}});
        }
        assert!(validate_input_schema("deep", &deep).is_err());
    }

    #[test]
    fn non_object_root_fails_closed() {
        assert!(validate_input_schema("array", &json!({"type": "array"})).is_err());
    }

    proptest! {
        #[test]
        fn generated_exact_integer_contracts_accept_only_matching_objects(
            property in "[A-Za-z][A-Za-z0-9_]{0,31}",
            value in any::<i64>()
        ) {
            let schema = json!({
                "type": "object",
                "properties": { (property.clone()): { "type": "integer" } },
                "required": [property.clone()],
                "additionalProperties": false
            });
            prop_assert!(validate_input_schema("generated", &schema).is_ok());
            let valid = validate_arguments(
                "generated",
                &schema,
                &json!({(property.clone()): value}),
            );
            let wrong_type = validate_arguments(
                "generated",
                &schema,
                &json!({(property.clone()): "wrong"}),
            );
            let extra_property = validate_arguments(
                "generated",
                &schema,
                &json!({(property): value, "undeclared": true}),
            );
            prop_assert!(valid.is_ok());
            prop_assert!(wrong_type.is_err());
            prop_assert!(extra_property.is_err());
        }
    }
}
