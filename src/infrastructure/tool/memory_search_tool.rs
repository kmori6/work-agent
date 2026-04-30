use crate::domain::error::tool_error::ToolError;
use crate::domain::port::tool::Tool;
use crate::domain::port::tool::ToolOutput;
use crate::domain::service::memory_index_service::MemoryIndexService;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;

pub struct MemorySearchTool {
    memory_index_service: Arc<MemoryIndexService>,
}

impl MemorySearchTool {
    pub fn new(memory_index_service: Arc<MemoryIndexService>) -> Self {
        Self {
            memory_index_service,
        }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search Commander journal memory by semantic similarity."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "description": "Maximum number of results. Default is 5."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'query'".into()))?;

        let limit = match arguments.get("limit") {
            None | Some(Value::Null) => DEFAULT_LIMIT,
            Some(value) => {
                let limit = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'limit' must be an integer".into())
                })? as usize;

                if limit == 0 || limit > MAX_LIMIT {
                    return Err(ToolError::InvalidArguments(format!(
                        "'limit' must be between 1 and {MAX_LIMIT}"
                    )));
                }

                limit
            }
        };

        let results = self
            .memory_index_service
            .search(query, limit)
            .await
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to search memory: {err}")))?;

        let results = results
            .into_iter()
            .map(|result| {
                json!({
                    "path": result.path,
                    "chunk_index": result.chunk_index,
                    "content": result.content
                })
            })
            .collect::<Vec<_>>();

        Ok(ToolOutput::success(json!({
            "results": results
        })))
    }
}
