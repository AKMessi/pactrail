use pactrail_core::{Capability, PermissionSet, PolicyDecision, ResourceScope};

/// Pure policy evaluator for tool capability requests.
#[derive(Clone, Debug)]
pub struct PolicyEngine {
    permissions: PermissionSet,
}

impl PolicyEngine {
    /// Creates a policy from the task's explicit permissions.
    #[must_use]
    pub const fn new(permissions: PermissionSet) -> Self {
        Self { permissions }
    }

    /// Allows workspace I/O, requires process approval, and denies external effects.
    #[must_use]
    pub fn local_default() -> Self {
        let mut permissions = PermissionSet::default();
        permissions.allow.insert(Capability::FileRead);
        permissions.allow.insert(Capability::FileWrite);
        permissions.deny.insert(Capability::Network);
        permissions.deny.insert(Capability::SecretUse);
        permissions.deny.insert(Capability::ExternalWrite);
        Self { permissions }
    }

    /// Evaluates one narrowly scoped effect request.
    #[must_use]
    pub fn evaluate(
        &self,
        capability: &Capability,
        resource: impl Into<String>,
        actor_fingerprint: Option<String>,
    ) -> PolicyDecision {
        let scope = ResourceScope {
            capability: capability.clone(),
            resource: resource.into(),
            actor_fingerprint,
        };
        if self.permissions.deny.contains(capability) {
            return PolicyDecision::Deny {
                reason: format!("{capability} is denied by the task contract"),
            };
        }
        if self.permissions.allow.contains(capability) {
            return PolicyDecision::Allow {
                scope,
                reason: format!("{capability} is allowed by the task contract"),
            };
        }
        PolicyDecision::Ask {
            scope,
            reason: format!("{capability} requires a scoped approval"),
        }
    }

    /// Returns a copy with one additional explicit grant.
    #[must_use]
    pub fn with_allowed(mut self, capability: Capability) -> Self {
        self.permissions.deny.remove(&capability);
        self.permissions.allow.insert(capability);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_requires_process_approval() {
        let decision = PolicyEngine::local_default().evaluate(
            &Capability::ProcessSpawn,
            "cargo",
            Some("run_process".to_owned()),
        );
        assert!(matches!(decision, PolicyDecision::Ask { .. }));
    }

    #[test]
    fn explicit_denial_wins() {
        let mut permissions = PermissionSet::default();
        permissions.allow.insert(Capability::Network);
        permissions.deny.insert(Capability::Network);
        let decision =
            PolicyEngine::new(permissions).evaluate(&Capability::Network, "example.com", None);
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }
}
