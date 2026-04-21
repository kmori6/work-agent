use crate::application::error::digest_usecase_error::DigestUsecaseError;
use crate::domain::model::message::Message;
use crate::domain::model::role::Role;
use crate::domain::port::llm_provider::LlmProvider;
use serde::Deserialize;

const MODEL: &str = "global.anthropic.claude-haiku-4-5-20251001-v1:0";

const TRANSLATE_PROMPT: &str = "\
Translate the following Markdown document into Japanese. \
For each paper's body text (the paragraph below the URL), condense it into 3-4 sentences \
following this structure: background → problem → proposed approach → results. \
Keep all Markdown formatting (headings, URLs, horizontal rules) intact. \
Output only the translated Markdown, with no additional commentary.";

#[derive(Debug, Deserialize)]
struct PaperEntry {
    paper: Paper,
}

#[derive(Debug, Deserialize)]
struct Paper {
    id: String,
    title: String,
    summary: String,
}

#[derive(Debug)]
pub struct RunDigestInput {
    pub date: String,
}

#[derive(Debug)]
pub struct RunDigestOutput {
    pub report: String,
}

pub struct DigestUsecase<L> {
    llm_provider: L,
}

impl<L: LlmProvider> DigestUsecase<L> {
    pub fn new(llm_provider: L) -> Self {
        Self { llm_provider }
    }

    pub async fn run(&self, input: RunDigestInput) -> Result<RunDigestOutput, DigestUsecaseError> {
        let url = format!(
            "https://huggingface.co/api/daily_papers?date={}",
            input.date
        );

        let entries: Vec<PaperEntry> = reqwest::get(&url)
            .await
            .map_err(|e| DigestUsecaseError::Fetch(e.to_string()))?
            .json()
            .await
            .map_err(|e| DigestUsecaseError::Fetch(e.to_string()))?;

        let english_report = build_report(&input.date, &entries);

        let report = self
            .llm_provider
            .response(
                vec![
                    Message::text(Role::System, TRANSLATE_PROMPT),
                    Message::text(Role::User, english_report),
                ],
                MODEL,
            )
            .await
            .map_err(|e| DigestUsecaseError::Translate(e.to_string()))?;

        Ok(RunDigestOutput { report })
    }
}

fn build_report(date: &str, entries: &[PaperEntry]) -> String {
    let mut out = format!("# Daily Papers - {date}\n\n");

    for entry in entries {
        out.push_str(&format!("## {}\n\n", entry.paper.title));
        out.push_str(&format!(
            "https://huggingface.co/papers/{}\n\n",
            entry.paper.id
        ));
        out.push_str(&format!("{}\n\n", entry.paper.summary.trim()));
        out.push_str("---\n\n");
    }

    out
}
