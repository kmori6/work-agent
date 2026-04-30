use crate::application::error::survey_usecase_error::SurveyUsecaseError;
use crate::domain::model::message::{Message, MessageContent};
use crate::domain::model::role::Role;
use crate::domain::port::llm_provider::{LlmProvider, StructuredOutputSchema};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use uuid::Uuid;

#[derive(Debug)]
pub struct RunSurveyInput {
    pub source: String,
}

#[derive(Debug)]
pub struct RunSurveyOutput {
    pub report: String,
}

pub struct SurveyUsecase<L> {
    llm_provider: L,
}

impl<L: LlmProvider> SurveyUsecase<L> {
    pub fn new(llm_provider: L) -> Self {
        Self { llm_provider }
    }

    pub async fn run(&self, input: RunSurveyInput) -> Result<RunSurveyOutput, SurveyUsecaseError> {
        let text = self.extract_text(&input.source).await?;

        let messages = vec![
            Message::new(
                Role::System,
                vec![MessageContent::InputText(SYSTEM_PROMPT.to_string())],
            )
            .map_err(|e| SurveyUsecaseError::LlmClient(e.to_string()))?,
            Message::input_text(format!(
                "Please read the following paper and summarize it.\n\n{text}"
            ))
            .map_err(|e| SurveyUsecaseError::LlmClient(e.to_string()))?,
        ];

        let json = self
            .llm_provider
            .response_with_structure(
                messages,
                StructuredOutputSchema {
                    name: "paper_survey".to_string(),
                    description: Some("Academic paper survey summary".to_string()),
                    schema: paper_summary_schema(),
                },
                "global.anthropic.claude-sonnet-4-6",
            )
            .await
            .map_err(|e| SurveyUsecaseError::LlmClient(e.to_string()))?;

        let report = json_to_markdown(&input.source, &json);
        Ok(RunSurveyOutput { report })
    }

    async fn extract_text(&self, source: &str) -> Result<String, SurveyUsecaseError> {
        let path = if source.starts_with("http://") || source.starts_with("https://") {
            self.download_to_tempfile(source).await?
        } else {
            PathBuf::from(source)
        };

        self.run_markitdown(&path).await
    }

    async fn download_to_tempfile(&self, url: &str) -> Result<PathBuf, SurveyUsecaseError> {
        let bytes = reqwest::get(url)
            .await
            .map_err(|e| SurveyUsecaseError::Download(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| SurveyUsecaseError::Download(e.to_string()))?;

        // Write a tmp file -> /tmp/commander_survey_<uuid>.pdf
        let path = std::env::temp_dir().join(format!("commander_survey_{}.pdf", Uuid::new_v4()));
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| SurveyUsecaseError::PdfRead(e.to_string()))?;

        Ok(path)
    }

    async fn run_markitdown(&self, path: &Path) -> Result<String, SurveyUsecaseError> {
        let output = Command::new("markitdown")
            .arg(path)
            .output()
            .await
            .map_err(|e| SurveyUsecaseError::PdfRead(format!("failed to run markitdown: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SurveyUsecaseError::PdfRead(format!(
                "markitdown failed: {stderr}"
            )));
        }

        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        Ok(text)
    }
}

const SYSTEM_PROMPT: &str = r#"You are an expert academic paper analyst.
Read the given paper and extract the following information.
All output values must be written in Japanese.

1. background
   The general research context of the field.
   What is the domain, and why does it matter?

2. problem
   The specific unsolved problem or gap this paper addresses.
   Typically introduced with "However," or "Despite this,".
   Be concrete about what was missing before this work.

3. method
   What the paper proposes and why it works.
   Focus on the core idea and intuition — not implementation details like
   layer counts or hyperparameters. A reader should understand
   "what this method is doing and why it's clever" from this field alone.

4. experiments
   What tasks or datasets were used, which baselines were compared,
   and what the key quantitative results were (e.g. BLEU score, accuracy).
   Include enough numbers to judge whether the improvement is meaningful.

5. contribution
   The novelty of this paper in one or two sentences.
   What does this paper do for the first time, or better than prior work?

6. discussion
   What the results imply beyond the paper itself.
   Broader impact, limitations the authors acknowledge,
   and what future directions are opened up.

7. related_papers
   Papers closely related to this work that are worth reading next.
   Extract from citations. Include title and authors if mentioned.
   Limit to the most important 3-5 papers."#;

fn paper_summary_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title":        { "type": "string" },
            "authors":      { "type": "array", "items": { "type": "string" } },
            "year":         { "type": "string" },
            "background":   { "type": "string" },
            "problem":      { "type": "string" },
            "method":       { "type": "string" },
            "experiments":  { "type": "string" },
            "contribution": { "type": "string" },
            "discussion":   { "type": "string" },
            "related_papers": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "title":   { "type": "string" },
                        "authors": { "type": "string" }
                    },
                    "required": ["title", "authors"]
                }
            }
        },
        "required": ["title","authors","year","background","problem","method","experiments","contribution","discussion","related_papers"]
    })
}

fn json_to_markdown(source: &str, json: &serde_json::Value) -> String {
    let title = json["title"].as_str().unwrap_or("-");
    let authors = json["authors"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let year = json["year"].as_str().unwrap_or("-");
    let background = json["background"].as_str().unwrap_or("-");
    let problem = json["problem"].as_str().unwrap_or("-");
    let method = json["method"].as_str().unwrap_or("-");
    let experiments = json["experiments"].as_str().unwrap_or("-");
    let contribution = json["contribution"].as_str().unwrap_or("-");
    let discussion = json["discussion"].as_str().unwrap_or("-");
    let related = json["related_papers"]
        .as_array()
        .map(|papers| {
            papers
                .iter()
                .map(|p| {
                    let t = p["title"].as_str().unwrap_or("-");
                    let a = p["authors"].as_str().unwrap_or("");
                    if a.is_empty() {
                        format!("- {t}")
                    } else {
                        format!("- {t} ({a})")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    format!(
        "# {title} ({year})\n\
         > {authors}\n\
         > source: `{source}`\n\n\
         ## Background\n{background}\n\n\
         ## Problem\n{problem}\n\n\
         ## Method\n{method}\n\n\
         ## Experiments\n{experiments}\n\n\
         ## Contribution\n{contribution}\n\n\
         ## Discussion\n{discussion}\n\n\
         ## Related Papers\n{related}\n"
    )
}
