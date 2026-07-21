#![no_main]

use libfuzzer_sys::fuzz_target;
use pactrail_mcp::{validate_arguments, validate_input_schema, validate_output_schema};
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let Ok(value) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    let _input = validate_input_schema("fuzz", &value);
    let _output = validate_output_schema("fuzz", &value);
    let _arguments = validate_arguments("fuzz", &value, &value);
});
