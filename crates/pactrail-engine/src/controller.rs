use std::collections::BTreeSet;

use pactrail_models::{ConversationItem, Role, ToolResult};
use pactrail_tools::ToolDescriptor;

const MAX_DISCOVERY_TURNS: u16 = 6;
const MIN_RESERVED_ACTION_TURNS: u16 = 4;
const SEMANTIC_STEERING_THRESHOLD: u16 = 2;
const PHASE_MARKER: &str = "Pactrail controller phase:";

/// Model-neutral phase enforced by Pactrail's deterministic run controller.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ControllerPhase {
    /// Gather bounded repository evidence before changing code.
    Investigating,
    /// Produce the smallest coherent candidate from evidence already gathered.
    Implementing,
    /// Inspect, test, and repair an existing isolated candidate.
    Validating,
    /// Produce an evidence-backed answer for a read-only task.
    Synthesizing,
}

impl ControllerPhase {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Investigating => "investigating",
            Self::Implementing => "implementing",
            Self::Validating => "validating",
            Self::Synthesizing => "synthesizing",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GoalIntent {
    Informational,
    Change,
}

#[derive(Debug)]
pub(crate) struct TurnControl {
    pub(crate) phase: ControllerPhase,
    pub(crate) phase_turn: u16,
    pub(crate) phase_limit: u16,
    pub(crate) tools: Vec<ToolDescriptor>,
    pub(crate) allowed_tool_names: BTreeSet<String>,
    pub(crate) phase_changed: bool,
    pub(crate) prompt: Option<String>,
    pub(crate) reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProgressAssessment {
    pub(crate) novel_evidence: usize,
    pub(crate) candidate_changed: bool,
    pub(crate) no_progress_turns: u16,
}

/// Deterministic control plane around the provider-neutral model loop.
///
/// The kernel deliberately derives durable state from the checkpointed
/// conversation and isolated candidate. Resuming therefore cannot forget tool
/// evidence or bypass a phase restriction, and the checkpoint schema does not
/// need a compatibility-breaking controller payload.
pub(crate) struct ControllerKernel {
    intent: GoalIntent,
    max_turns: u16,
    discovery_limit: u16,
    phase: Option<ControllerPhase>,
    phase_turn: u16,
    announced_phases: BTreeSet<ControllerPhase>,
    evidence_digests: BTreeSet<String>,
    no_progress_turns: u16,
}

impl ControllerKernel {
    #[cfg(test)]
    pub(crate) fn restore(goal: &str, max_turns: u16, conversation: &[ConversationItem]) -> Self {
        Self::restore_with_discovery_cap(goal, max_turns, MAX_DISCOVERY_TURNS, conversation)
    }

    pub(crate) fn restore_with_discovery_cap(
        goal: &str,
        max_turns: u16,
        discovery_turn_cap: u16,
        conversation: &[ConversationItem],
    ) -> Self {
        let mut evidence_digests = BTreeSet::new();
        let mut announced_phases = BTreeSet::new();
        let mut phase = None;
        let mut phase_turn = 0_u16;
        let mut pending_tool_turn_novel = None;
        let mut no_progress_turns = 0_u16;
        for item in conversation {
            match item {
                ConversationItem::ToolResult(result) if !result.is_error => {
                    let novel = evidence_digests.insert(tool_result_digest(result));
                    if let Some(turn_novel) = pending_tool_turn_novel.as_mut() {
                        *turn_novel |= novel;
                    }
                }
                ConversationItem::Message(message) if message.role == Role::System => {
                    finish_tool_turn(&mut pending_tool_turn_novel, &mut no_progress_turns);
                    if let Some(parsed_phase) = parse_phase_marker(&message.content) {
                        announced_phases.insert(parsed_phase);
                        phase_turn = 0;
                        // The latest marker is authoritative for the model
                        // turns that follow it in the checkpointed sequence.
                        phase = Some(parsed_phase);
                    }
                }
                ConversationItem::AssistantToolCalls { .. } => {
                    finish_tool_turn(&mut pending_tool_turn_novel, &mut no_progress_turns);
                    phase_turn = phase_turn.saturating_add(1);
                    pending_tool_turn_novel = Some(false);
                }
                ConversationItem::Message(message) if message.role == Role::Assistant => {
                    finish_tool_turn(&mut pending_tool_turn_novel, &mut no_progress_turns);
                    phase_turn = phase_turn.saturating_add(1);
                }
                ConversationItem::Message(_)
                | ConversationItem::UserContent(_)
                | ConversationItem::ToolResult(_) => {
                    if !matches!(item, ConversationItem::ToolResult(_)) {
                        finish_tool_turn(&mut pending_tool_turn_novel, &mut no_progress_turns);
                    }
                }
            }
        }
        finish_tool_turn(&mut pending_tool_turn_novel, &mut no_progress_turns);
        Self {
            intent: classify_goal(goal),
            max_turns,
            discovery_limit: discovery_turn_limit(max_turns, discovery_turn_cap),
            phase,
            phase_turn,
            announced_phases,
            evidence_digests,
            no_progress_turns,
        }
    }

    pub(crate) const fn intent(&self) -> GoalIntent {
        self.intent
    }

    pub(crate) fn before_turn(
        &mut self,
        turn: u16,
        candidate_present: bool,
        all_tools: &[ToolDescriptor],
    ) -> TurnControl {
        let phase = self.phase_for(turn, candidate_present);
        let phase_changed = self.phase != Some(phase);
        self.phase_turn = if phase_changed {
            1
        } else {
            self.phase_turn.saturating_add(1)
        };
        self.phase = Some(phase);
        let phase_turn = self.phase_turn;
        let phase_limit = self.phase_limit(phase);
        let tools = select_tools(all_tools, phase, phase_turn, self.no_progress_turns);
        let allowed_tool_names = tools.iter().map(|tool| tool.name.clone()).collect();
        let prompt = if self.announced_phases.contains(&phase) {
            None
        } else {
            self.announced_phases.insert(phase);
            Some(phase_prompt(
                phase,
                phase_turn,
                self.max_turns.saturating_sub(turn),
            ))
        };
        TurnControl {
            phase,
            phase_turn,
            phase_limit,
            tools,
            allowed_tool_names,
            phase_changed,
            prompt,
            reason: self.phase_reason(phase, candidate_present),
        }
    }

    pub(crate) fn observe_turn(&mut self, results: &[(ToolResult, bool)]) -> ProgressAssessment {
        let mut novel_evidence = 0_usize;
        let candidate_changed = results.iter().any(|(_, changed)| *changed);
        for (result, _) in results {
            if !result.is_error && self.evidence_digests.insert(tool_result_digest(result)) {
                novel_evidence = novel_evidence.saturating_add(1);
            }
        }
        if candidate_changed || novel_evidence > 0 {
            self.no_progress_turns = 0;
        } else {
            self.no_progress_turns = self.no_progress_turns.saturating_add(1);
        }
        ProgressAssessment {
            novel_evidence,
            candidate_changed,
            no_progress_turns: self.no_progress_turns,
        }
    }

    pub(crate) fn steering_prompt(&self) -> Option<String> {
        (self.no_progress_turns >= SEMANTIC_STEERING_THRESHOLD).then(|| {
            if self.intent == GoalIntent::Change {
                format!(
                    "Pactrail progress controller: {turns} consecutive tool turns produced neither new evidence nor a candidate change. Stop repeating equivalent observations. State the concrete defect hypothesis internally, then make the smallest supported candidate change or finish with a precise blocker. Do not resume broad exploration.",
                    turns = self.no_progress_turns
                )
            } else {
                format!(
                    "Pactrail progress controller: {turns} consecutive tool turns produced no new evidence. Answer the original question from the evidence already gathered, or state exactly what remains unknown. Do not repeat equivalent reads.",
                    turns = self.no_progress_turns
                )
            }
        })
    }

    fn phase_for(&self, turn: u16, candidate_present: bool) -> ControllerPhase {
        match self.intent {
            GoalIntent::Informational => {
                if self.phase == Some(ControllerPhase::Synthesizing)
                    || self.no_progress_turns >= SEMANTIC_STEERING_THRESHOLD
                    || turn.saturating_add(1) >= self.max_turns
                {
                    ControllerPhase::Synthesizing
                } else {
                    ControllerPhase::Investigating
                }
            }
            GoalIntent::Change if candidate_present => ControllerPhase::Validating,
            GoalIntent::Change
                if matches!(
                    self.phase,
                    Some(ControllerPhase::Implementing | ControllerPhase::Validating)
                ) =>
            {
                ControllerPhase::Implementing
            }
            GoalIntent::Change
                if turn >= self.discovery_limit
                    || self.no_progress_turns >= SEMANTIC_STEERING_THRESHOLD =>
            {
                ControllerPhase::Implementing
            }
            GoalIntent::Change => ControllerPhase::Investigating,
        }
    }

    const fn phase_limit(&self, phase: ControllerPhase) -> u16 {
        match phase {
            ControllerPhase::Investigating => self.discovery_limit,
            ControllerPhase::Implementing | ControllerPhase::Validating => {
                self.max_turns.saturating_sub(self.discovery_limit)
            }
            ControllerPhase::Synthesizing => 1,
        }
    }

    fn phase_reason(&self, phase: ControllerPhase, candidate_present: bool) -> String {
        match phase {
            ControllerPhase::Investigating => format!(
                "bounded discovery turn; {} implementation/verification turn(s) reserved",
                self.max_turns.saturating_sub(self.discovery_limit)
            ),
            ControllerPhase::Implementing if self.no_progress_turns > 0 => format!(
                "semantic progress stalled for {} turn(s); broad discovery is disabled",
                self.no_progress_turns
            ),
            ControllerPhase::Implementing => {
                "discovery budget exhausted; broad discovery is disabled".to_owned()
            }
            ControllerPhase::Validating if candidate_present => {
                "an isolated candidate exists; tools are focused on review, checks, and repair"
                    .to_owned()
            }
            ControllerPhase::Validating => "candidate validation is active".to_owned(),
            ControllerPhase::Synthesizing => {
                "read-only evidence is sufficient or the final answer reserve is active".to_owned()
            }
        }
    }
}

fn discovery_turn_limit(max_turns: u16, discovery_turn_cap: u16) -> u16 {
    if max_turns <= MIN_RESERVED_ACTION_TURNS {
        return 0;
    }
    (max_turns / 3)
        .clamp(2, MAX_DISCOVERY_TURNS.min(discovery_turn_cap.max(2)))
        .min(max_turns.saturating_sub(MIN_RESERVED_ACTION_TURNS))
}

fn select_tools(
    all_tools: &[ToolDescriptor],
    phase: ControllerPhase,
    phase_turn: u16,
    no_progress_turns: u16,
) -> Vec<ToolDescriptor> {
    if phase == ControllerPhase::Investigating {
        return all_tools.to_vec();
    }
    if phase == ControllerPhase::Synthesizing {
        return Vec::new();
    }

    let focused_read_available = phase_turn == 1 && no_progress_turns == 0;
    all_tools
        .iter()
        .filter(|tool| {
            !tool.annotations.read_only
                || tool.name == "workspace_changes"
                || (focused_read_available && tool.name == "read_file")
        })
        .cloned()
        .collect()
}

fn finish_tool_turn(pending: &mut Option<bool>, no_progress_turns: &mut u16) {
    if let Some(novel) = pending.take() {
        if novel {
            *no_progress_turns = 0;
        } else {
            *no_progress_turns = no_progress_turns.saturating_add(1);
        }
    }
}

fn phase_prompt(phase: ControllerPhase, phase_turn: u16, turns_remaining: u16) -> String {
    let instruction = match phase {
        ControllerPhase::Investigating => "Gather only task-relevant evidence.",
        ControllerPhase::Implementing if phase_turn == 1 => {
            "The bounded discovery phase is complete. Broad listing, search, history, and memory tools are unavailable. Use at most one focused read if indispensable, then make the smallest coherent candidate change supported by the evidence. If no safe change can be identified, return a precise blocker instead of continuing to browse."
        }
        ControllerPhase::Implementing => {
            "The focused-read allowance is exhausted. Make the smallest coherent candidate change now, or return a precise blocker. Do not request unavailable discovery tools."
        }
        ControllerPhase::Validating => {
            "An isolated candidate now exists. Inspect the candidate, run the most relevant available checks, and repair failures. Broad discovery is unavailable because verification and finalization turns are reserved."
        }
        ControllerPhase::Synthesizing => {
            "Tool access is disabled for this bounded synthesis turn. Answer the original informational request using only evidence already present. Distinguish observed facts from inference and state any remaining uncertainty."
        }
    };
    format!(
        "{PHASE_MARKER} {}. {instruction} {turns_remaining} model turn(s) remain.",
        phase.label()
    )
}

fn parse_phase_marker(content: &str) -> Option<ControllerPhase> {
    let phase = content.strip_prefix(PHASE_MARKER)?.trim_start();
    if phase.starts_with(ControllerPhase::Investigating.label()) {
        Some(ControllerPhase::Investigating)
    } else if phase.starts_with(ControllerPhase::Implementing.label()) {
        Some(ControllerPhase::Implementing)
    } else if phase.starts_with(ControllerPhase::Validating.label()) {
        Some(ControllerPhase::Validating)
    } else if phase.starts_with(ControllerPhase::Synthesizing.label()) {
        Some(ControllerPhase::Synthesizing)
    } else {
        None
    }
}

fn tool_result_digest(result: &ToolResult) -> String {
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, result.name.as_bytes());
    let bytes = serde_json::to_vec(&result.content).unwrap_or_default();
    hash_field(&mut hasher, &bytes);
    hasher.finalize().to_hex().to_string()
}

fn hash_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
    hasher.update(value);
}

pub(crate) fn classify_goal(goal: &str) -> GoalIntent {
    let normalized = goal.trim().to_ascii_lowercase();
    let words = normalized
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let first = words.first().copied().unwrap_or_default();
    if matches!(
        first,
        "what" | "whats" | "why" | "how" | "who" | "where" | "when"
    ) {
        return GoalIntent::Informational;
    }
    let requests_change = words.iter().any(|word| {
        matches!(
            *word,
            "add"
                | "build"
                | "change"
                | "create"
                | "delete"
                | "edit"
                | "fix"
                | "generate"
                | "implement"
                | "migrate"
                | "modify"
                | "refactor"
                | "remove"
                | "rename"
                | "update"
                | "write"
        )
    });
    if requests_change {
        return GoalIntent::Change;
    }
    if matches!(
        first,
        "analyze" | "describe" | "explain" | "inspect" | "review" | "show" | "summarize"
    ) || normalized.ends_with('?')
    {
        GoalIntent::Informational
    } else {
        GoalIntent::Change
    }
}

#[cfg(test)]
mod tests {
    use pactrail_core::Capability;
    use pactrail_models::{Message, ToolCall, ToolResult};
    use pactrail_tools::ToolAnnotations;
    use serde_json::json;

    use super::*;

    fn descriptor(name: &str, annotations: ToolAnnotations) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_owned(),
            description: name.to_owned(),
            input_schema: json!({"type": "object"}),
            required_capability: if annotations.read_only {
                Capability::FileRead
            } else {
                Capability::FileWrite
            },
            annotations,
        }
    }

    #[test]
    fn reserves_action_turns_even_for_small_budgets() {
        assert_eq!(discovery_turn_limit(1, MAX_DISCOVERY_TURNS), 0);
        assert_eq!(discovery_turn_limit(4, MAX_DISCOVERY_TURNS), 0);
        assert_eq!(discovery_turn_limit(8, MAX_DISCOVERY_TURNS), 2);
        assert_eq!(discovery_turn_limit(16, MAX_DISCOVERY_TURNS), 5);
        assert_eq!(discovery_turn_limit(24, MAX_DISCOVERY_TURNS), 6);
        assert_eq!(discovery_turn_limit(24, 2), 2);
    }

    #[test]
    fn adaptive_discovery_cap_moves_compact_profiles_to_action_early() {
        let mut controller =
            ControllerKernel::restore_with_discovery_cap("Implement a focused fix", 24, 2, &[]);
        let tools = Vec::<ToolDescriptor>::new();
        assert_eq!(
            controller.before_turn(0, false, &tools).phase,
            ControllerPhase::Investigating
        );
        assert_eq!(
            controller.before_turn(1, false, &tools).phase,
            ControllerPhase::Investigating
        );
        assert_eq!(
            controller.before_turn(2, false, &tools).phase,
            ControllerPhase::Implementing
        );
    }

    #[test]
    fn implementation_phase_removes_broad_discovery_tools() {
        let tools = vec![
            descriptor("search", ToolAnnotations::READ_ONLY),
            descriptor("read_file", ToolAnnotations::READ_ONLY),
            descriptor("workspace_changes", ToolAnnotations::READ_ONLY),
            descriptor("edit_file", ToolAnnotations::WORKSPACE_MUTATION),
        ];
        let first = select_tools(&tools, ControllerPhase::Implementing, 1, 0);
        assert_eq!(
            first
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["read_file", "workspace_changes", "edit_file"]
        );
        let later = select_tools(&tools, ControllerPhase::Implementing, 2, 0);
        assert_eq!(
            later
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["workspace_changes", "edit_file"]
        );
    }

    #[test]
    fn semantic_progress_ignores_equivalent_results_with_different_call_ids() {
        let mut kernel = ControllerKernel::restore("fix the parser", 16, &[]);
        let first = ToolResult {
            call_id: "one".to_owned(),
            name: "read_file".to_owned(),
            content: json!({"path": "src/lib.rs", "text": "fn parse() {}"}),
            is_error: false,
        };
        let second = ToolResult {
            call_id: "two".to_owned(),
            ..first.clone()
        };
        assert_eq!(kernel.observe_turn(&[(first, false)]).novel_evidence, 1);
        let repeated = kernel.observe_turn(&[(second, false)]);
        assert_eq!(repeated.novel_evidence, 0);
        assert_eq!(repeated.no_progress_turns, 1);
    }

    #[test]
    fn candidate_mutation_is_progress_even_when_output_repeats() {
        let mut kernel = ControllerKernel::restore("fix the parser", 16, &[]);
        let result = ToolResult {
            call_id: "one".to_owned(),
            name: "edit_file".to_owned(),
            content: json!({"changed": true}),
            is_error: false,
        };
        kernel.observe_turn(&[(result.clone(), false)]);
        let assessment = kernel.observe_turn(&[(result, true)]);
        assert!(assessment.candidate_changed);
        assert_eq!(assessment.no_progress_turns, 0);
    }

    #[test]
    fn change_tasks_enter_implementation_after_discovery_budget() {
        let tools = vec![
            descriptor("search", ToolAnnotations::READ_ONLY),
            descriptor("edit_file", ToolAnnotations::WORKSPACE_MUTATION),
        ];
        let mut kernel = ControllerKernel::restore("fix the parser", 16, &[]);
        assert_eq!(
            kernel.before_turn(4, false, &tools).phase,
            ControllerPhase::Investigating
        );
        let controlled = kernel.before_turn(5, false, &tools);
        assert_eq!(controlled.phase, ControllerPhase::Implementing);
        assert!(controlled.tools.iter().all(|tool| tool.name != "search"));
        assert!(controlled.prompt.is_some());
    }

    #[test]
    fn an_early_intervention_never_reopens_broad_discovery() {
        let tools = vec![
            descriptor("search", ToolAnnotations::READ_ONLY),
            descriptor("read_file", ToolAnnotations::READ_ONLY),
            descriptor("edit_file", ToolAnnotations::WORKSPACE_MUTATION),
        ];
        let failed = ToolResult {
            call_id: "failed".to_owned(),
            name: "search".to_owned(),
            content: json!({"error": "no match"}),
            is_error: true,
        };
        let evidence = ToolResult {
            call_id: "evidence".to_owned(),
            name: "read_file".to_owned(),
            content: json!({"text": "relevant"}),
            is_error: false,
        };
        let mut kernel = ControllerKernel::restore("fix the parser", 16, &[]);

        assert_eq!(
            kernel.before_turn(0, false, &tools).phase,
            ControllerPhase::Investigating
        );
        kernel.observe_turn(&[(failed.clone(), false)]);
        kernel.before_turn(1, false, &tools);
        kernel.observe_turn(&[(failed, false)]);
        assert_eq!(
            kernel.before_turn(2, false, &tools).phase,
            ControllerPhase::Implementing
        );
        kernel.observe_turn(&[(evidence, false)]);
        let next = kernel.before_turn(3, false, &tools);
        assert_eq!(next.phase, ControllerPhase::Implementing);
        assert!(next.tools.iter().all(|tool| tool.name != "search"));
    }

    #[test]
    fn resume_reconstructs_phase_turn_and_semantic_stall_from_conversation() {
        let tools = vec![
            descriptor("search", ToolAnnotations::READ_ONLY),
            descriptor("edit_file", ToolAnnotations::WORKSPACE_MUTATION),
        ];
        let conversation = vec![
            ConversationItem::Message(Message::system(phase_prompt(
                ControllerPhase::Implementing,
                1,
                6,
            ))),
            ConversationItem::AssistantToolCalls {
                text: String::new(),
                calls: vec![ToolCall {
                    id: "failed".to_owned(),
                    name: "search".to_owned(),
                    arguments: json!({"query": "missing"}),
                    extensions: serde_json::Map::new(),
                }],
            },
            ConversationItem::ToolResult(ToolResult {
                call_id: "failed".to_owned(),
                name: "search".to_owned(),
                content: json!({"error": "no match"}),
                is_error: true,
            }),
        ];

        let mut restored = ControllerKernel::restore("fix the parser", 8, &conversation);
        let controlled = restored.before_turn(1, false, &tools);
        assert_eq!(controlled.phase, ControllerPhase::Implementing);
        assert_eq!(controlled.phase_turn, 2);
        assert_eq!(restored.no_progress_turns, 1);
        assert!(controlled.prompt.is_none());
        assert!(controlled.tools.iter().all(|tool| tool.name != "search"));
    }
}
