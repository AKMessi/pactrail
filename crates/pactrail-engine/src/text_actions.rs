use pactrail_models::{ConversationItem, Message, ToolCall};
use pactrail_tools::ToolDescriptor;
use serde::Deserialize;
use serde_json::{Value, json};

const OPEN: &str = "<pactrail_action>";
const CLOSE: &str = "</pactrail_action>";
const MAX_ACTION_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextAction {
    name: String,
    arguments: Value,
}

pub(crate) fn catalog_prompt(tools: &[ToolDescriptor]) -> Result<String, serde_json::Error> {
    let catalog = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "Pactrail text action protocol: to call one tool, reply with exactly <pactrail_action>{{\"name\":\"tool_name\",\"arguments\":{{...}}}}</pactrail_action> and no other text. Only one action is allowed per turn. When finished, reply with normal prose and no action tag. Never claim a tool result you have not received. Available tools: {}",
        serde_json::to_string(&catalog)?
    ))
}

pub(crate) fn parse_action(text: &str, turn: u16) -> Result<Option<ToolCall>, String> {
    let text = text.trim();
    if !text.contains(OPEN) && !text.contains(CLOSE) {
        return Ok(None);
    }
    let body = text
        .strip_prefix(OPEN)
        .and_then(|body| body.strip_suffix(CLOSE))
        .ok_or_else(|| "text action must be the entire response".to_owned())?;
    if body.len() > MAX_ACTION_BYTES {
        return Err("text action exceeds the input limit".to_owned());
    }
    let action: TextAction = serde_json::from_str(body)
        .map_err(|error| format!("text action JSON is invalid: {error}"))?;
    if action.name.is_empty() || !action.arguments.is_object() {
        return Err("text action requires a tool name and object arguments".to_owned());
    }
    Ok(Some(ToolCall {
        id: format!("text-action-{turn}"),
        name: action.name,
        arguments: action.arguments,
        extensions: serde_json::Map::new(),
    }))
}

pub(crate) fn transport_conversation(
    conversation: &[ConversationItem],
) -> Result<Vec<ConversationItem>, String> {
    conversation
        .iter()
        .map(|item| match item {
            ConversationItem::AssistantToolCalls { text, calls } => {
                if calls.len() != 1 {
                    return Err("text action conversation requires one call per turn".to_owned());
                }
                let call = calls
                    .first()
                    .ok_or_else(|| "empty tool call batch".to_owned())?;
                let action = json!({"name": call.name, "arguments": call.arguments});
                let serialized =
                    serde_json::to_string(&action).map_err(|error| error.to_string())?;
                let mut content = format!("{OPEN}{serialized}{CLOSE}");
                if !text.is_empty() {
                    content.push('\n');
                    content.push_str(text);
                }
                Ok(ConversationItem::Message(Message::assistant(content)))
            }
            ConversationItem::ToolResult(result) => {
                Ok(ConversationItem::Message(Message::user(format!(
                    "Pactrail tool result: {}",
                    serde_json::to_string(result).map_err(|error| error.to_string())?
                ))))
            }
            item => Ok(item.clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_one_exact_action_and_rejects_embedded_or_unsafe_shapes() {
        let action = parse_action(
            "<pactrail_action>{\"name\":\"read_file\",\"arguments\":{\"path\":\"src/lib.rs\"}}</pactrail_action>",
            3,
        )
        .unwrap_or_else(|error| unreachable!("valid action: {error}"))
        .unwrap_or_else(|| unreachable!("action missing"));
        assert_eq!(action.id, "text-action-3");
        assert_eq!(action.name, "read_file");
        assert_eq!(action.arguments["path"], "src/lib.rs");
        assert!(parse_action("answer", 0).is_ok_and(|action| action.is_none()));
        assert!(parse_action("hello <pactrail_action>{}</pactrail_action>", 0).is_err());
        assert!(
            parse_action(
                "<pactrail_action>{\"name\":\"x\",\"arguments\":[]}</pactrail_action>",
                0
            )
            .is_err()
        );
        assert!(
            parse_action(
                "<pactrail_action>{\"name\":\"x\",\"arguments\":{},\"extra\":1}</pactrail_action>",
                0
            )
            .is_err()
        );
    }
}
