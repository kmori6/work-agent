use crate::application::error::digest_usecase_error::DigestUsecaseError;
use crate::domain::model::message::{Message, MessageContent};
use crate::domain::model::role::Role;
use crate::domain::port::llm_provider::LlmProvider;

const MODEL: &str = "global.anthropic.claude-haiku-4-5-20251001-v1:0";

const PROMPT_BUSINESS_GLOBAL: &str = "\
You are given the raw HTML of two English tech newsletters for the same date: \
TLDR AI (AI/ML focus) and TLDR Tech (broader tech industry). \
Extract all articles from both. Skip sponsors and advertisements. \
If the same story appears in both newsletters, merge them into one entry. \
Select the 20 most important and newsworthy articles. \
Prioritize AI, developer tools, major business deals, and significant product launches. \
IMPORTANT: Do NOT use sub-headings, category labels, or any ## headings other than \"## Business (Global)\". \
All articles must appear as a single flat list directly under the \"## Business (Global)\" heading. \
Using sub-headings will be considered an error.

Produce a Markdown section in exactly this format:

## Business (Global)

### {title translated to Japanese}
{url}
{2-3 sentence summary in Japanese}

---

Output only the Markdown, no additional commentary.";

const PROMPT_BUSINESS_DOMESTIC: &str = "\
You are given the XML of two Japanese tech news RSS feeds: \
ITmedia News (general tech) and ITmedia AI+ (AI-focused). \
Extract all articles. If the same story appears in both feeds, merge into one entry. \
Select the 20 most important and newsworthy articles. \
Prioritize AI, major business news, technology policy, and significant product announcements. \
Exclude trivial news, weather events, and unrelated lifestyle content. \
IMPORTANT: Do NOT use sub-headings, category labels, or any ## headings other than \"## Business (Domestic)\". \
All articles must appear as a single flat list directly under the \"## Business (Domestic)\" heading. \
Using sub-headings will be considered an error.

Produce a Markdown section in exactly this format:

## Business (Domestic)

### {article title in Japanese}
{url}
{2-3 sentence summary in Japanese based on the title and description}

---

Output only the Markdown, no additional commentary.";

const PROMPT_TECH: &str = "\
You are given the raw HTML of the TLDR Dev newsletter \
(developer tools, OSS, frontend, backend, cloud infrastructure). \
Extract all articles. Skip sponsors and advertisements. \
Focus on technical and developer content. \
Skip articles that are primarily company/business news (funding, acquisitions, executive changes) \
as those are covered in the Business section.

Produce a Markdown section in exactly this format:

## Tech

### {title translated to Japanese}
{url}
{2-3 sentence summary in Japanese}

---

Output only the Markdown, no additional commentary.";

const PROMPT_ACADEMIA: &str = "\
You are given a list of HuggingFace Daily Papers (academic AI/ML research). \
Select the 20 most significant papers. \
Prioritize papers with novel methods, strong empirical results, or broad practical impact. \
Exclude niche domain-specific papers (e.g., biology, agriculture, medical statistics) \
unless they demonstrate a broadly applicable AI/ML technique.

Produce a Markdown section in exactly this format:

## Academia

### {title translated to Japanese}
{url}
{2 sentence summary in Japanese: proposed approach and key results}

---

Output only the Markdown, no additional commentary.";

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

impl<L: LlmProvider + Sync> DigestUsecase<L> {
    pub fn new(llm_provider: L) -> Self {
        Self { llm_provider }
    }

    pub async fn run(&self, input: RunDigestInput) -> Result<RunDigestOutput, DigestUsecaseError> {
        let client = reqwest::Client::new();

        // Fetch all sources in parallel
        let url_tldr_ai = format!("https://tldr.tech/ai/{}", input.date);
        let url_tldr_tech = format!("https://tldr.tech/tech/{}", input.date);
        let url_tldr_dev = format!("https://tldr.tech/dev/{}", input.date);
        let url_papers = format!(
            "https://huggingface.co/api/daily_papers?date={}",
            input.date
        );
        let (tldr_ai, tldr_tech, tldr_dev, itmedia_news_xml, itmedia_ai_xml, papers_json) = tokio::try_join!(
            fetch_text(&client, &url_tldr_ai),
            fetch_text(&client, &url_tldr_tech),
            fetch_text(&client, &url_tldr_dev),
            fetch_text(&client, "https://rss.itmedia.co.jp/rss/2.0/news_bursts.xml"),
            fetch_text(&client, "https://rss.itmedia.co.jp/rss/2.0/aiplus.xml"),
            fetch_text(&client, &url_papers),
        )?;

        // Summarize each section in parallel via separate LLM calls
        let (business_global, business_domestic, tech, academia) = tokio::try_join!(
            self.llm_call(
                PROMPT_BUSINESS_GLOBAL,
                format!("## TLDR AI\n\n{tldr_ai}\n\n## TLDR Tech\n\n{tldr_tech}"),
            ),
            self.llm_call(
                PROMPT_BUSINESS_DOMESTIC,
                format!(
                    "## ITmedia News RSS\n\n{itmedia_news_xml}\n\n## ITmedia AI+ RSS\n\n{itmedia_ai_xml}"
                ),
            ),
            self.llm_call(PROMPT_TECH, tldr_dev),
            self.llm_call(PROMPT_ACADEMIA, papers_json),
        )?;

        let report = format!(
            "# Daily Digest - {date}\n\n{business_global}\n\n{business_domestic}\n\n{tech}\n\n{academia}",
            date = input.date,
        );

        Ok(RunDigestOutput { report })
    }

    async fn llm_call(
        &self,
        system_prompt: &str,
        user_content: String,
    ) -> Result<String, DigestUsecaseError> {
        let messages = vec![
            Message::new(
                Role::System,
                vec![MessageContent::InputText(system_prompt.to_string())],
            )
            .map_err(|e| DigestUsecaseError::Translate(e.to_string()))?,
            Message::input_text(user_content)
                .map_err(|e| DigestUsecaseError::Translate(e.to_string()))?,
        ];

        self.llm_provider
            .response(messages, MODEL)
            .await
            .map_err(|e| DigestUsecaseError::Translate(e.to_string()))
    }
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, DigestUsecaseError> {
    client
        .get(url)
        .send()
        .await
        .map_err(|e| DigestUsecaseError::Fetch(e.to_string()))?
        .text()
        .await
        .map_err(|e| DigestUsecaseError::Fetch(e.to_string()))
}
