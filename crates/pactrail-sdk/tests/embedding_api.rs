use pactrail_sdk::model::{FinishReason, Usage};
use pactrail_sdk::prelude::*;
use serde_json::{Value, json};

struct CustomModel {
    provider: String,
    model: String,
    capabilities: ModelCapabilities,
}

#[async_trait]
impl ModelDriver for CustomModel {
    fn name(&self) -> &str {
        &self.provider
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn capabilities(&self) -> &ModelCapabilities {
        &self.capabilities
    }

    async fn invoke(&self, _request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        Ok(ModelResponse {
            text: "complete".to_owned(),
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Complete,
            usage: Usage::default(),
            provider_request_id: None,
            extensions: serde_json::Map::new(),
        })
    }
}

struct CustomTool;

#[async_trait]
impl Tool for CustomTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "custom_workspace_fact".to_owned(),
            description: "Return one deterministic workspace fact.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            required_capability: Capability::FileRead,
            annotations: ToolAnnotations::READ_ONLY,
        }
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        _input: Value,
    ) -> Result<ToolOutput, ToolError> {
        context.authorize(&Capability::FileRead, ".", "custom_workspace_fact")?;
        Ok(ToolOutput {
            content: json!({"fact": "embedded extension executed"}),
            summary: "returned one custom fact".to_owned(),
            observed_effects: vec!["fs.read:.".to_owned()],
            succeeded: true,
            truncated: false,
        })
    }
}

#[test]
fn external_model_and_tool_compose_with_the_real_kernel() {
    let model = CustomModel {
        provider: "custom-provider".to_owned(),
        model: "custom-model".to_owned(),
        capabilities: ModelCapabilities::default(),
    };
    let mut registry = ToolRegistry::new();
    registry
        .register(CustomTool)
        .unwrap_or_else(|error| unreachable!("custom tool registration: {error}"));
    let mut permissions = PermissionSet::default();
    permissions.allow.insert(Capability::FileRead);
    let policy = PolicyEngine::new(permissions);

    let _engine = RunEngine::new(&model, &registry, &policy);
    assert_eq!(model.name(), "custom-provider");
    assert_eq!(registry.descriptors()[0].name, "custom_workspace_fact");
    assert_eq!(pactrail_sdk::SDK_API_REVISION, 6);
}
