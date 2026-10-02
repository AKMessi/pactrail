//! Recovery advice is explanatory; the engine remains the recovery authority.
use pactrail_engine::EngineError;
use pactrail_models::ModelError;

use crate::commands::CliError;

pub(crate) struct Advice {
    pub title: &'static str,
    pub next: &'static str,
}

pub(crate) fn advice(error: &CliError) -> Advice {
    match error {
        CliError::RunFailed { source, .. } => engine_advice(source),
        CliError::Engine(source) => engine_advice(source),
        CliError::Model(source) => model_advice(source),
        CliError::Io { .. } | CliError::Store(_) | CliError::Receipt(_) => storage_advice(),
        CliError::Transaction(_) => workspace_advice(),
        _ => Advice {
            title: "Request could not complete",
            next: "Read the exact error above. Use /help for command syntax; no automatic retry has been started.",
        },
    }
}

fn engine_advice(error: &EngineError) -> Advice {
    match error {
        EngineError::Model(error) => model_advice(error),
        EngineError::BudgetExceeded { .. }
        | EngineError::CostBudgetExceeded { .. }
        | EngineError::CostReservationExceeded { .. }
        | EngineError::WallTimeExceeded { .. }
        | EngineError::MaxTurns(_) => Advice {
            title: "Execution limit reached",
            next: "Inspect /trace before starting another task. Continue does not reset this run's usage or lift its contract limits. /retry restores the brief for editing; it does not execute it.",
        },
        EngineError::Cancelled => Advice {
            title: "Run stopped",
            next: "Inspect /trace and /runs for retained state. Stopped runs are not automatically resumed or applied; write a new task explicitly if needed.",
        },
        EngineError::ResumeRejected(_) => Advice {
            title: "Recovery refused safely",
            next: "The engine could not validate this checkpoint. Inspect /trace. Do not delete checkpoints, edit the event chain, or copy files over the workspace to bypass the refusal.",
        },
        EngineError::ContextWindow(_) => Advice {
            title: "Context could not fit safely",
            next: "Inspect /trace and narrow the task or correct the model's configured context limits. /retry restores the brief without dispatching. No history was silently dropped to force a request.",
        },
        EngineError::Store(_)
        | EngineError::ObservationArtifact(_)
        | EngineError::Checkpoint(_)
        | EngineError::Receipt(_) => storage_advice(),
        EngineError::Transaction(_) => workspace_advice(),
        EngineError::ProcessCleanup(_) => Advice {
            title: "Process cleanup needs attention",
            next: "Inspect the exact cleanup error and active processes. Do not retry uncertain process effects. Candidate work remains subject to the normal review boundary.",
        },
        EngineError::Protocol(_) | EngineError::Stalled { .. } => Advice {
            title: "Model could not complete the task",
            next: "Inspect /trace for completed actions. Continue asks the engine to validate a safe checkpoint; it can refuse. /retry restores the brief for a smaller, explicit task. Partial model text is not a completed result.",
        },
        _ => Advice {
            title: "Execution could not start or finish",
            next: "Correct the exact configuration, permission, or tool error above before retrying. No permission or budget was increased automatically.",
        },
    }
}

fn model_advice(error: &ModelError) -> Advice {
    match error {
        ModelError::Provider {
            status: 401 | 403, ..
        } => Advice {
            title: "Provider access rejected",
            next: "Check the configured provider, model, and key environment variable. /status shows configuration without exposing the key. Correct access before retrying; repeated attempts will not fix authentication.",
        },
        ModelError::InvalidRequest(_)
        | ModelError::Provider {
            status: 400 | 404 | 422,
            ..
        } => Advice {
            title: "Provider configuration rejected",
            next: "Check /status and the exact provider error. Confirm the endpoint, model ID, and supported capabilities before retrying.",
        },
        ModelError::Provider { status: 429, .. } => Advice {
            title: "Provider rate limit reached",
            next: "The request did not complete within its retry policy. Wait for the provider's limit to clear. Continue validates the existing checkpoint; it does not promise a free request or reset budgets.",
        },
        ModelError::Transport(_)
        | ModelError::Json(_)
        | ModelError::MalformedResponse(_)
        | ModelError::Provider { .. } => Advice {
            title: "Provider response unavailable",
            next: "Continue asks the engine to validate the last safe checkpoint. A failed provider request may still have been billed; cost-capped recovery can be refused. Completed tools are not intentionally replayed.",
        },
        ModelError::ResponseTooLarge { .. } => Advice {
            title: "Provider response exceeded its safety limit",
            next: "Inspect /trace and correct the endpoint or reduce the task. The response-size limit was not bypassed and no partial response was accepted.",
        },
    }
}

fn storage_advice() -> Advice {
    Advice {
        title: "Durable storage needs attention",
        next: "Check free disk space, permissions, and the exact path in the error. Preserve the state directory for diagnosis. Do not delete or modify the event database, receipts, or checkpoints to force recovery.",
    }
}

fn workspace_advice() -> Advice {
    Advice {
        title: "Workspace could not be handled safely",
        next: "Check the exact path, ignore rules, symlinks, permissions, or source-drift error. Do not copy an isolated candidate over the source to bypass review. Correct the workspace condition before retrying.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_budget_and_unsafe_resume_have_distinct_actions() {
        let auth = advice(&CliError::Model(ModelError::Provider {
            status: 401,
            message: "secret must never be repeated in advice".to_owned(),
        }));
        assert_eq!(auth.title, "Provider access rejected");
        assert!(!auth.next.contains("secret"));
        let budget = engine_advice(&EngineError::MaxTurns(2));
        assert!(budget.next.contains("does not reset"));
        let unsafe_resume =
            engine_advice(&EngineError::ResumeRejected("pending effects".to_owned()));
        assert_eq!(unsafe_resume.title, "Recovery refused safely");
        assert!(!unsafe_resume.next.contains("/apply"));
    }
}
