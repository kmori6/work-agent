use crate::domain::model::tool_execution_policy::ToolExecutionPolicy;
use crate::domain::model::tool_execution_rule::ToolExecutionRuleAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionDecision {
    Allow,
    Ask,
    Deny,
}

impl ToolExecutionDecision {
    pub fn decide(policy: ToolExecutionPolicy, rule: Option<ToolExecutionRuleAction>) -> Self {
        match (policy, rule) {
            (_, Some(ToolExecutionRuleAction::Deny)) => Self::Deny,
            (ToolExecutionPolicy::ConfirmEveryTime, _) => Self::Ask,
            (_, Some(ToolExecutionRuleAction::Allow)) => Self::Allow,
            (_, Some(ToolExecutionRuleAction::Ask)) => Self::Ask,
            (ToolExecutionPolicy::Auto, None) => Self::Allow,
            (ToolExecutionPolicy::Ask, None) => Self::Ask,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }
}
