use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::{ToolCall, ToolExecutionResult, ToolSpec};
use crate::domain::model::tool_execution_decision::ToolExecutionDecision;
use crate::domain::model::tool_execution_policy::ToolExecutionPolicy;
use crate::domain::model::tool_execution_rule::ToolExecutionRuleAction;
use crate::domain::port::tool::Tool;
use crate::domain::repository::tool_execution_rule_repository::ToolExecutionRuleRepository;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolRuleSummary {
    pub tool_name: String,
    pub policy: ToolExecutionPolicy,
    pub rule: Option<ToolExecutionRuleAction>,
    pub action: ToolExecutionDecision,
    pub source: ToolRuleSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRuleSource {
    Saved,
    Default,
}

impl ToolRuleSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Default => "default",
        }
    }
}

#[derive(Clone)]
pub struct ToolExecutor {
    tools: Vec<Arc<dyn Tool>>,
    rule_repository: Arc<dyn ToolExecutionRuleRepository>,
}

impl ToolExecutor {
    pub fn new(
        tools: Vec<Arc<dyn Tool>>,
        rule_repository: Arc<dyn ToolExecutionRuleRepository>,
    ) -> Self {
        Self {
            tools,
            rule_repository,
        }
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|tool| tool.spec()).collect()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .map(|tool| tool.name().to_string())
            .collect()
    }

    pub async fn execute(&self, call: ToolCall) -> Result<ToolExecutionResult, ToolError> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == call.name)
            .ok_or_else(|| ToolError::UnknownTool(call.name.clone()))?;

        tool.execute(call.arguments).await
    }

    pub async fn decide_execution(
        &self,
        call: &ToolCall,
    ) -> Result<ToolExecutionDecision, ToolError> {
        let policy = self.check_execution_policy(call)?;
        let rule = self
            .rule_repository
            .find_by_tool_name(&call.name)
            .await?
            .map(|rule| rule.action);

        Ok(ToolExecutionDecision::decide(policy, rule))
    }

    pub fn check_execution_policy(
        &self,
        call: &ToolCall,
    ) -> Result<ToolExecutionPolicy, ToolError> {
        self.tools
            .iter()
            .find(|tool| tool.name() == call.name)
            .map(|tool| tool.execution_policy(&call.arguments))
            .ok_or_else(|| ToolError::UnknownTool(call.name.clone()))
    }

    pub async fn tool_rule_summaries(&self) -> Result<Vec<ToolRuleSummary>, ToolError> {
        let mut summaries = Vec::with_capacity(self.tools.len());
        for tool in &self.tools {
            let rule = self.rule_repository.find_by_tool_name(tool.name()).await?;
            let policy = tool.execution_policy(&serde_json::Value::Null);
            let rule_action = rule.as_ref().map(|rule| rule.action);
            let action =
                ToolExecutionDecision::decide(policy, rule.as_ref().map(|rule| rule.action));
            let source = if rule.is_some() {
                ToolRuleSource::Saved
            } else {
                ToolRuleSource::Default
            };

            summaries.push(ToolRuleSummary {
                tool_name: tool.name().to_string(),
                policy,
                rule: rule_action,
                action,
                source,
            });
        }
        Ok(summaries)
    }
}
