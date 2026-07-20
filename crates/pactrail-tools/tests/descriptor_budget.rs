use pactrail_tools::builtin_registry;

const MAX_BUILTIN_TOOLS: usize = 32;
const MAX_SERIALIZED_DESCRIPTOR_BYTES: usize = 32 * 1024;
const MAX_SINGLE_DESCRIPTOR_BYTES: usize = 8 * 1024;
const MAX_SCHEMA_DEPTH: usize = 24;

#[test]
fn builtin_descriptor_catalog_stays_within_the_model_budget() {
    let descriptors = builtin_registry()
        .unwrap_or_else(|error| unreachable!("built-in registry: {error}"))
        .descriptors();
    assert!(descriptors.len() <= MAX_BUILTIN_TOOLS);
    let serialized = serde_json::to_vec(&descriptors)
        .unwrap_or_else(|error| unreachable!("serialize descriptors: {error}"));
    assert!(serialized.len() <= MAX_SERIALIZED_DESCRIPTOR_BYTES);

    for descriptor in descriptors {
        let encoded = serde_json::to_vec(&descriptor)
            .unwrap_or_else(|error| unreachable!("serialize {}: {error}", descriptor.name));
        assert!(
            encoded.len() <= MAX_SINGLE_DESCRIPTOR_BYTES,
            "{} descriptor is {} bytes",
            descriptor.name,
            encoded.len()
        );
        assert!(!descriptor.description.trim().is_empty());
        assert!(json_depth(&descriptor.input_schema) <= MAX_SCHEMA_DEPTH);
    }
}

fn json_depth(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .map(json_depth)
            .max()
            .unwrap_or_default()
            .saturating_add(1),
        serde_json::Value::Object(values) => values
            .values()
            .map(json_depth)
            .max()
            .unwrap_or_default()
            .saturating_add(1),
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => 1,
    }
}
