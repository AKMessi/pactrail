#![no_main]

use libfuzzer_sys::fuzz_target;
use pactrail_core::agent::AgentRunConfig;
use pactrail_engine::agents::AgentRuntime;
use pactrail_models::latent::{LatentDescriptor, LatentState};

fuzz_target!(|data: &[u8]| {
    if data.len() > 1_048_576 { return; }
    if let Ok(config) = serde_json::from_slice::<AgentRunConfig>(data) {
        let _validated = config.validate();
    }
    if let Ok(runtime) = serde_json::from_slice::<AgentRuntime>(data) {
        let _validated = runtime.validate(runtime.run_id);
    }
    if let Ok(descriptor) = serde_json::from_slice::<LatentDescriptor>(data) {
        let _validated = descriptor.validate(descriptor.run_id, &descriptor.model, 1_048_576);
        // The input, never an untrusted dimension, determines this allocation.
        let _sealed = LatentState::from_bytes(descriptor.clone(), data.to_vec().into(), descriptor.run_id, &descriptor.model, 1_048_576);
    }
});
