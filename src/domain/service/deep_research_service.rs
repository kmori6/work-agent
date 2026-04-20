use crate::application::error::llm_client_error::LlmClientError;
use crate::domain::error::deep_research_error::DeepResearchError;
use crate::domain::model::message::Message;
use crate::domain::model::role::Role;
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::port::llm_provider::StructuredOutputSchema;
use crate::domain::port::search_provider::{SearchDocument, SearchProvider};
use serde_json::json;

const DEFAULT_MODEL: &str = "global.anthropic.claude-haiku-4-5-20251001-v1:0";
const DEFAULT_MAX_REVISION_STEPS: usize = 3; // original paper used 20 steps
const DEFAULT_NUM_INITIAL_ANSWER_STATES: usize = 3; // paper: n_a = 3
const DEFAULT_NUM_INITIAL_QUESTION_STATES: usize = 5; // paper: n_q = 5

#[derive(Debug, Clone)]
struct ResearchPlan {
    sections: Vec<String>,
}

#[derive(Debug, Clone)]
struct DraftReport {
    revision: usize,
    content: String,
}

#[derive(Debug, Clone)]
struct ResearchQuestion {
    focus: String,
    text: String,
}

#[derive(Debug, Clone)]
struct ResearchAnswer {
    summary: String,
}

#[derive(Debug, Clone)]
struct QuestionAnswerPair {
    question: ResearchQuestion,
    answer: ResearchAnswer,
}

pub struct DeepResearchService<L, S> {
    llm_provider: L,
    search_provider: S,
    model: String,
    max_revision_steps: usize,
}

/// Proposed in R. Han et al., "Deep Researcher with Test-Time Diffusion"
impl<L, S> DeepResearchService<L, S>
where
    L: LlmProvider,
    S: SearchProvider,
{
    pub fn new(llm_provider: L, search_provider: S) -> Self {
        Self {
            llm_provider,
            search_provider,
            model: DEFAULT_MODEL.to_string(),
            max_revision_steps: DEFAULT_MAX_REVISION_STEPS,
        }
    }

    pub async fn research(&self, query: String) -> Result<String, DeepResearchError> {
        print_progress_stage("Generating research plan...");
        let plan = self.generate_research_plan(&query).await?;
        print_progress_stage("Research plan generated");

        print_progress_stage("Generating preliminary draft...");
        let mut draft = self.generate_preliminary_draft(&query, &plan).await?;
        print_progress_stage("Preliminary draft generated");
        let mut history = Vec::new();
        let mut revision_history = vec![draft.clone()];

        for step_index in 0..self.max_revision_steps {
            let question = self
                .generate_next_question(&query, &plan, &draft, &history)
                .await?;
            let answer = self.retrieve_answer(&query, &question).await?;
            history.push(QuestionAnswerPair { question, answer });

            draft = self.denoise_draft(&query, &plan, &draft, &history).await?;
            revision_history.push(draft.clone());

            let should_exit = self.exit_loop(&query, &plan, &draft, &history).await?;
            if let Some(latest_step) = history.last() {
                print_iteration_summary(
                    step_index + 1,
                    &latest_step.question,
                    &latest_step.answer,
                    should_exit,
                );
            }

            if should_exit {
                break;
            }
        }

        print_progress_stage("Generating final report...");
        let final_report = self
            .generate_final_report(&query, &plan, &draft, &history, &revision_history)
            .await?;
        print_progress_stage("Final report generated");

        Ok(final_report)
    }

    async fn generate_research_plan(&self, query: &str) -> Result<ResearchPlan, LlmClientError> {
        let initial_plan = self.generate_initial_research_plan(query).await?;
        let critique = self.critique_research_plan(query, &initial_plan).await?;
        self.revise_research_plan(query, &initial_plan, &critique)
            .await
    }

    async fn generate_preliminary_draft(
        &self,
        query: &str,
        plan: &ResearchPlan,
    ) -> Result<DraftReport, LlmClientError> {
        let plan_text = plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let messages = vec![
            Message::text(
                Role::System,
                "You are a research writing assistant. Write an initial draft report for the user's query. The draft must be written in Japanese. It should be coherent, useful, and explicitly tentative where facts may need verification. Do not invent citations, URLs, or precise facts you are not confident about.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Write a preliminary draft report for the following query.\n\nQuery: {query}\n\nResearch plan:\n{plan_text}\n\nRequirements:\n- Write in Japanese\n- Organize the draft around the research plan\n- Cover every plan item at least once\n- Use the model's internal knowledge only\n- If something may require verification, write it cautiously rather than pretending it is confirmed\n- Do not include citations or URLs\n- Return only the draft report in Markdown"
                ),
            ),
        ];

        let content = self.llm_provider.response(messages, &self.model).await?;

        Ok(DraftReport {
            revision: 0,
            content: content.trim().to_string(),
        })
    }

    async fn generate_next_question(
        &self,
        query: &str,
        plan: &ResearchPlan,
        draft: &DraftReport,
        history: &[QuestionAnswerPair],
    ) -> Result<ResearchQuestion, LlmClientError> {
        let candidates = self
            .generate_question_candidates(query, plan, draft, history)
            .await?;

        self.select_best_question(query, plan, draft, history, &candidates)
            .await
    }

    async fn retrieve_answer(
        &self,
        query: &str,
        question: &ResearchQuestion,
    ) -> Result<ResearchAnswer, DeepResearchError> {
        let documents = self.search_provider.search(&question.text).await?;

        if documents.is_empty() {
            return Ok(ResearchAnswer {
                summary: format!(
                    "No relevant search results found for the question: {}",
                    question.focus
                ),
            });
        }

        let candidates = self
            .generate_answer_candidates(query, question, &documents)
            .await?;

        let merged = self.merge_answers(query, question, &candidates).await?;
        Ok(merged)
    }

    async fn denoise_draft(
        &self,
        query: &str,
        plan: &ResearchPlan,
        previous_draft: &DraftReport,
        history: &[QuestionAnswerPair],
    ) -> Result<DraftReport, LlmClientError> {
        let latest_step = history.last().ok_or_else(|| {
            LlmClientError::ResponseParse(
                "history must contain at least one step before revision".to_string(),
            )
        })?;

        let plan_text = plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let history_text = history
            .iter()
            .enumerate()
            .map(|(index, step)| {
                format!(
                    "{}.\nFocus: {}\nQuestion: {}\nAnswer: {}",
                    index + 1,
                    step.question.focus,
                    step.question.text,
                    step.answer.summary
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let messages = vec![
            Message::text(
                Role::System,
                "You are a research draft revision assistant. Revise the current draft report using the newly retrieved answer. The revised draft must be written in Japanese. Improve accuracy, reduce uncertainty where the new evidence helps, remove redundancy, and keep the report coherent. Do not invent facts beyond the provided draft and retrieved answers.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Revise the current draft report for the following query.\n\nQuery: {query}\n\nResearch plan:\n{plan_text}\n\nCurrent draft (revision {}):\n{}\n\nLatest retrieved question-answer pair:\nFocus: {}\nQuestion: {}\nAnswer: {}\n\nAll question-answer history:\n{history_text}\n\nRequirements:\n- Write in Japanese\n- Revise the full draft, not just an appended note\n- Integrate the new answer into the most relevant part of the draft\n- Keep the structure aligned with the research plan\n- Remove redundancy where possible\n- If uncertainty remains, express it cautiously\n- Do not include citations or URLs\n- Return only the revised draft report in Markdown",
                    previous_draft.revision,
                    previous_draft.content,
                    latest_step.question.focus,
                    latest_step.question.text,
                    latest_step.answer.summary
                ),
            ),
        ];

        let revised_content = self.llm_provider.response(messages, &self.model).await?;

        Ok(DraftReport {
            revision: previous_draft.revision + 1,
            content: revised_content.trim().to_string(),
        })
    }

    async fn exit_loop(
        &self,
        query: &str,
        plan: &ResearchPlan,
        draft: &DraftReport,
        history: &[QuestionAnswerPair],
    ) -> Result<bool, LlmClientError> {
        let plan_text = plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let history_text = if history.is_empty() {
            "No previous question-answer history yet.".to_string()
        } else {
            history
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    format!(
                        "{}.\nFocus: {}\nQuestion: {}\nAnswer: {}",
                        index + 1,
                        step.question.focus,
                        step.question.text,
                        step.answer.summary
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        let schema = StructuredOutputSchema {
            name: "exit_loop_decision".to_string(),
            description: Some(
                "A coverage judgment for whether the current research loop can stop.".to_string(),
            ),
            schema: json!({
                "type": "object",
                "properties": {
                    "should_exit": { "type": "boolean" },
                    "reason": {
                        "type": "string",
                        "minLength": 1
                    },
                    "uncovered_sections": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "minLength": 1
                        }
                    }
                },
                "required": ["should_exit", "reason", "uncovered_sections"],
                "additionalProperties": false
            }),
        };

        let messages = vec![
            Message::text(
                Role::System,
                "You are a research coverage evaluator. Decide whether the research plan has been adequately covered by the current draft and the accumulated question-answer history. Evaluate coverage section by section before making the final decision.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Evaluate whether the research loop can stop.\n\nQuery: {query}\n\nResearch plan:\n{plan_text}\n\nCurrent draft:\n{}\n\nQuestion-answer history:\n{history_text}\n\nRequirements:\n- Evaluate each research plan section against the current draft and history\n- List every section that is still insufficiently covered in `uncovered_sections`\n- Be conservative; if any important section remains underdeveloped, do not stop\n- `should_exit` should be true only when the plan is adequately covered overall\n- Return JSON only",
                    draft.content
                ),
            ),
        ];

        let value = self
            .llm_provider
            .response_with_structure(messages, schema, &self.model)
            .await?;

        let should_exit = value
            .get("should_exit")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| {
                LlmClientError::ResponseParse("failed to decode exit_loop.should_exit".to_string())
            })?;

        let reason = value
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                LlmClientError::ResponseParse("failed to decode exit_loop.reason".to_string())
            })?;

        let uncovered_sections: Vec<String> =
            serde_json::from_value(value.get("uncovered_sections").cloned().ok_or_else(|| {
                LlmClientError::ResponseParse(
                    "failed to decode exit_loop.uncovered_sections".to_string(),
                )
            })?)
            .map_err(|err| {
                LlmClientError::ResponseParse(format!(
                    "failed to parse exit_loop uncovered_sections: {err}"
                ))
            })?;

        let uncovered_sections = uncovered_sections
            .into_iter()
            .map(|section| section.trim().to_string())
            .filter(|section| !section.is_empty())
            .collect::<Vec<_>>();

        if !uncovered_sections.is_empty() {
            println!(
                "exit_loop: continue; uncovered sections: {:?}; reason: {}",
                uncovered_sections, reason
            );
            return Ok(false);
        }

        println!("exit_loop: stop={should_exit}; reason: {reason}");
        Ok(should_exit)
    }

    async fn generate_final_report(
        &self,
        query: &str,
        plan: &ResearchPlan,
        draft: &DraftReport,
        history: &[QuestionAnswerPair],
        revision_history: &[DraftReport],
    ) -> Result<String, LlmClientError> {
        let initial_report = self
            .generate_initial_final_report(query, plan, draft, history, revision_history)
            .await?;
        let critique = self
            .critique_final_report(query, plan, &initial_report)
            .await?;
        self.revise_final_report(query, plan, &initial_report, &critique)
            .await
    }

    // helpers for each research step
    async fn generate_initial_research_plan(
        &self,
        query: &str,
    ) -> Result<ResearchPlan, LlmClientError> {
        let schema = StructuredOutputSchema {
            name: "initial_research_plan".to_string(),
            description: Some("A concise initial research plan.".to_string()),
            schema: json!({
                "type": "array",
                "items": {
                    "type": "string",
                    "minLength": 1
                },
                "minItems": 1
            }),
        };

        let messages = vec![
            Message::text(
                Role::System,
                "You are a research planning assistant. Create an initial research plan for the user's query.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Create an initial research plan for the following query.\n\nQuery: {query}\n\nRequirements:\n- Identify the necessary research angles needed to investigate the query thoroughly\n- Avoid redundant or overlapping items\n- Write each plan item concisely in Japanese\n- Return JSON only\n- The output format must be an array of strings"
                ),
            ),
        ];

        let value = self
            .llm_provider
            .response_with_structure(messages, schema, &self.model)
            .await?;

        let sections: Vec<String> = serde_json::from_value(value).map_err(|err| {
            LlmClientError::ResponseParse(format!(
                "failed to decode initial research plan structured output: {err}"
            ))
        })?;

        if sections.is_empty() {
            return Err(LlmClientError::ResponseParse(
                "research plan must contain at least one section".to_string(),
            ));
        }

        Ok(ResearchPlan { sections })
    }

    async fn critique_research_plan(
        &self,
        query: &str,
        plan: &ResearchPlan,
    ) -> Result<String, LlmClientError> {
        let plan_text = plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let schema = StructuredOutputSchema {
            name: "research_plan_critique".to_string(),
            description: Some("A critique of the initial research plan.".to_string()),
            schema: json!({
                "type": "object",
                "properties": {
                    "critique": {
                        "type": "string",
                        "minLength": 1
                    }
                },
                "required": ["critique"],
                "additionalProperties": false
            }),
        };

        let messages = vec![
            Message::text(
                Role::System,
                "You are a research plan critic. Review the research plan and identify weaknesses, omissions, redundancy, or poor prioritization.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Critique the following research plan.\n\nQuery: {query}\n\nResearch plan:\n{plan_text}\n\nRequirements:\n- Identify important missing research angles if any\n- Identify redundancy or overlap if any\n- Identify unclear or weakly phrased items if any\n- Be concise but specific\n- Return JSON only"
                ),
            ),
        ];

        let value = self
            .llm_provider
            .response_with_structure(messages, schema, &self.model)
            .await?;

        let critique = value
            .get("critique")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                LlmClientError::ResponseParse("failed to decode research plan critique".to_string())
            })?
            .to_string();

        Ok(critique)
    }

    async fn revise_research_plan(
        &self,
        query: &str,
        initial_plan: &ResearchPlan,
        critique: &str,
    ) -> Result<ResearchPlan, LlmClientError> {
        let initial_plan_text = initial_plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let schema = StructuredOutputSchema {
            name: "revised_research_plan".to_string(),
            description: Some("A revised research plan.".to_string()),
            schema: json!({
                "type": "array",
                "items": {
                    "type": "string",
                    "minLength": 1
                },
                "minItems": 1
            }),
        };

        let messages = vec![
            Message::text(
                Role::System,
                "You are a research planning assistant. Revise the initial research plan using the critique and produce a better plan.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Revise the following research plan.\n\nQuery: {query}\n\nInitial research plan:\n{initial_plan_text}\n\nCritique:\n{critique}\n\nRequirements:\n- Keep the plan concise\n- Remove redundancy\n- Add important missing angles if needed\n- Improve clarity and prioritization\n- Write each plan item in Japanese\n- Return JSON only\n- The output format must be an array of strings"
                ),
            ),
        ];

        let value = self
            .llm_provider
            .response_with_structure(messages, schema, &self.model)
            .await?;

        let sections: Vec<String> = serde_json::from_value(value).map_err(|err| {
            LlmClientError::ResponseParse(format!(
                "failed to decode revised research plan structured output: {err}"
            ))
        })?;

        if sections.is_empty() {
            return Err(LlmClientError::ResponseParse(
                "revised research plan must contain at least one section".to_string(),
            ));
        }

        Ok(ResearchPlan { sections })
    }

    async fn generate_answer_candidates(
        &self,
        query: &str,
        question: &ResearchQuestion,
        documents: &[SearchDocument],
    ) -> Result<Vec<ResearchAnswer>, DeepResearchError> {
        let documents_text = format_search_documents(documents);
        let mut candidates = Vec::with_capacity(DEFAULT_NUM_INITIAL_ANSWER_STATES);

        for candidate_index in 0..DEFAULT_NUM_INITIAL_ANSWER_STATES {
            let answer = self
                .generate_answer_candidate(query, question, &documents_text, candidate_index)
                .await?;
            candidates.push(answer);
        }

        Ok(candidates)
    }

    async fn generate_answer_candidate(
        &self,
        query: &str,
        question: &ResearchQuestion,
        documents_text: &str,
        candidate_index: usize,
    ) -> Result<ResearchAnswer, LlmClientError> {
        let variation_hint = match candidate_index {
            0 => {
                "Focus on the core factual answer. Prioritize the most direct and strongly supported information."
            }
            1 => {
                "Focus on breadth and surrounding context. Include complementary background, timeline, or related details when they are supported by the retrieved search results."
            }
            _ => {
                "Focus on uncertainty and edge cases. Highlight ambiguity, disagreement, missing evidence, or details that still require caution."
            }
        };

        let messages = vec![
            Message::text(
                Role::System,
                "You are a research answer synthesis assistant. Read the retrieved search results and produce a concise, accurate answer in Japanese. Do not invent facts beyond the provided search results. If the evidence is weak or incomplete, say so clearly.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Synthesize an answer for the following research question.\n\nUser query: {query}\nFocus: {}\nQuestion: {}\n\nRetrieved search results:\n{}\n\nAdditional instruction:\n- {}\n\nRequirements:\n- Write in Japanese\n- Use only the retrieved search results\n- Be concise but informative\n- Prefer concrete and verifiable details\n- If the evidence is incomplete or conflicting, state that clearly\n- Do not output Markdown headings",
                    question.focus, question.text, documents_text, variation_hint,
                ),
            ),
        ];

        let summary = self.llm_provider.response(messages, &self.model).await?;

        Ok(ResearchAnswer {
            summary: summary.trim().to_string(),
        })
    }

    async fn merge_answers(
        &self,
        query: &str,
        question: &ResearchQuestion,
        candidates: &[ResearchAnswer],
    ) -> Result<ResearchAnswer, LlmClientError> {
        let answer_list = format_answer_candidates(candidates);

        let messages = vec![
            Message::text(
                Role::System,
                "Your task is to research a topic and try to fulfill the user query. You are given a list of candidate answers. Combine them into a single answer so that it best fulfills the initial user query. If there is conflicting information, try to reconcile it in a logically sound way. Use only the candidate answers. The final answer must be written in Japanese.",
            ),
            Message::text(
                Role::User,
                format!(
                    "User query:\n{query}\n\nResearch focus:\n{}\n\nResearch question:\n{}\n\nCandidate answers:\n{}\n\nRequirements:\n- Write in Japanese\n- Merge the candidate answers into a single best answer\n- Use only the candidate answers above\n- Reconcile conflicts carefully when possible\n- If uncertainty remains, say so clearly\n- Do not output Markdown headings",
                    question.focus, question.text, answer_list
                ),
            ),
        ];

        let summary = self.llm_provider.response(messages, &self.model).await?;

        Ok(ResearchAnswer {
            summary: summary.trim().to_string(),
        })
    }

    async fn generate_question_candidates(
        &self,
        query: &str,
        plan: &ResearchPlan,
        draft: &DraftReport,
        history: &[QuestionAnswerPair],
    ) -> Result<Vec<ResearchQuestion>, LlmClientError> {
        let mut candidates = Vec::with_capacity(DEFAULT_NUM_INITIAL_QUESTION_STATES);

        for candidate_index in 0..DEFAULT_NUM_INITIAL_QUESTION_STATES {
            let candidate = self
                .generate_question_candidate(query, plan, draft, history, candidate_index)
                .await?;
            candidates.push(candidate);
        }

        Ok(candidates)
    }

    async fn generate_question_candidate(
        &self,
        query: &str,
        plan: &ResearchPlan,
        draft: &DraftReport,
        history: &[QuestionAnswerPair],
        candidate_index: usize,
    ) -> Result<ResearchQuestion, LlmClientError> {
        let schema = StructuredOutputSchema {
            name: "question_candidate".to_string(),
            description: Some(
                "A candidate next research question, including its focus and question text."
                    .to_string(),
            ),
            schema: json!({
                "type": "object",
                "properties": {
                    "focus": {
                        "type": "string",
                        "minLength": 1
                    },
                    "text": {
                        "type": "string",
                        "minLength": 1
                    }
                },
                "required": ["focus", "text"],
                "additionalProperties": false
            }),
        };

        let variation_hint = match candidate_index {
            0 => "Focus on the single most important missing factual gap in the current draft.",
            1 => "Focus on a broader contextual or background gap that would improve the report.",
            2 => "Focus on chronology, timeline, or change over time if relevant.",
            3 => {
                "Focus on uncertainty, ambiguity, disagreement, or weakly supported parts of the draft."
            }
            _ => {
                "Focus on a high-value angle that has not yet been explored in the previous question-answer history."
            }
        };

        let plan_text = plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let history_text = if history.is_empty() {
            "No previous question-answer history yet.".to_string()
        } else {
            history
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    format!(
                        "{}.\nFocus: {}\nQuestion: {}\nAnswer: {}",
                        index + 1,
                        step.question.focus,
                        step.question.text,
                        step.answer.summary
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        let messages = vec![
            Message::text(
                Role::System,
                "You are a research question generation assistant. Generate a candidate next research question that addresses an important remaining gap in the current draft.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Generate a candidate next research question for the following query.\n\nQuery: {query}\n\nResearch plan:\n{plan_text}\n\nCurrent draft (revision {}):\n{}\n\nPrevious question-answer history:\n{history_text}\n\nAdditional instruction:\n- {variation_hint}\n\nRequirements:\n- Identify an important remaining information gap in the current draft\n- Use the research plan to stay on track\n- Avoid repeating already covered questions\n- Return exactly one candidate question\n- Write both `focus` and `text` in Japanese\n- The question will be used for web search, so `text` must be concise and searchable\n- Prefer a single focused information need rather than a multi-part question\n- Avoid long explanations, subclauses, or excessive detail\n- Keep `text` under 200 characters\n- Return JSON only",
                    draft.revision, draft.content
                ),
            ),
        ];

        let value = self
            .llm_provider
            .response_with_structure(messages, schema, &self.model)
            .await?;

        let focus = value
            .get("focus")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                LlmClientError::ResponseParse(
                    "failed to decode question candidate focus".to_string(),
                )
            })?
            .to_string();

        let text = value
            .get("text")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                LlmClientError::ResponseParse(
                    "failed to decode question candidate text".to_string(),
                )
            })?
            .to_string();

        Ok(ResearchQuestion { focus, text })
    }

    async fn select_best_question(
        &self,
        query: &str,
        plan: &ResearchPlan,
        draft: &DraftReport,
        history: &[QuestionAnswerPair],
        candidates: &[ResearchQuestion],
    ) -> Result<ResearchQuestion, LlmClientError> {
        let schema = StructuredOutputSchema {
            name: "selected_question".to_string(),
            description: Some("The selected best candidate question.".to_string()),
            schema: json!({
                "type": "object",
                "properties": {
                    "selected_index": {
                        "type": "integer"
                    },
                    "reason": {
                        "type": "string",
                        "minLength": 1
                    }
                },
                "required": ["selected_index", "reason"],
                "additionalProperties": false
            }),
        };

        let plan_text = plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let history_text = if history.is_empty() {
            "No previous question-answer history yet.".to_string()
        } else {
            history
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    format!(
                        "{}.\nFocus: {}\nQuestion: {}\nAnswer: {}",
                        index + 1,
                        step.question.focus,
                        step.question.text,
                        step.answer.summary
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        let candidate_text = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                format!(
                    "[{}]\nFocus: {}\nQuestion: {}",
                    index, candidate.focus, candidate.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let messages = vec![
            Message::text(
                Role::System,
                "You are a research question selector. Choose the single best next question from the candidate list.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Select the best next research question for the following query.\n\nQuery: {query}\n\nResearch plan:\n{plan_text}\n\nCurrent draft (revision {}):\n{}\n\nPrevious question-answer history:\n{history_text}\n\nCandidate questions:\n{candidate_text}\n\nRequirements:\n- Choose the candidate that best addresses the most important remaining gap\n- Prefer novelty and coverage\n- Prefer a candidate that is concise and effective for web search\n- Avoid candidates that are overly long, multi-part, or difficult to search directly\n- Avoid repetition\n- Return JSON only",
                    draft.revision, draft.content
                ),
            ),
        ];

        let value = self
            .llm_provider
            .response_with_structure(messages, schema, &self.model)
            .await?;

        let selected_index = value
            .get("selected_index")
            .and_then(|value| value.as_i64())
            .ok_or_else(|| {
                LlmClientError::ResponseParse(
                    "failed to decode selected question index".to_string(),
                )
            })?;

        let selected_index = usize::try_from(selected_index).map_err(|_| {
            LlmClientError::ResponseParse(
                "selected question index must be non-negative".to_string(),
            )
        })?;

        candidates.get(selected_index).cloned().ok_or_else(|| {
            LlmClientError::ResponseParse(format!(
                "selected question index out of range: {selected_index}"
            ))
        })
    }

    async fn generate_initial_final_report(
        &self,
        query: &str,
        plan: &ResearchPlan,
        draft: &DraftReport,
        history: &[QuestionAnswerPair],
        revision_history: &[DraftReport],
    ) -> Result<String, LlmClientError> {
        let plan_text = plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let history_text = if history.is_empty() {
            "No question-answer history available.".to_string()
        } else {
            history
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    format!(
                        "{}.\nFocus: {}\nQuestion: {}\nAnswer: {}",
                        index + 1,
                        step.question.focus,
                        step.question.text,
                        step.answer.summary
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        let revision_history_text = format_revision_history(revision_history);

        let messages = vec![
            Message::text(
                Role::System,
                "You are a final report writing assistant. Write a comprehensive, coherent final report for the user's query. The report must be written in Japanese. Use the research plan, the current denoised draft, the accumulated question-answer history, and the revision history of the draft. Do not invent citations, URLs, or unsupported facts.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Write the final report for the following query.\n\nQuery: {query}\n\nResearch plan:\n{plan_text}\n\nCurrent denoised draft (revision {}):\n{}\n\nAccumulated question-answer history:\n{history_text}\n\nRevision history:\n{revision_history_text}\n\nRequirements:\n- Write in Japanese\n- Produce a coherent final report, not notes or bullet fragments\n- Use the research plan as the overall structure\n- Use the current draft as the primary backbone\n- Use the revision history to preserve how the report evolved over time\n- Incorporate the verified or synthesized information from the question-answer history\n- Prefer integrating information into a globally coherent report rather than writing each section independently\n- Remove redundancy and resolve inconsistencies where possible\n- If uncertainty remains, state it cautiously rather than pretending it is confirmed\n- Do not include citations or URLs\n- Return only the final report in Markdown",
                    draft.revision, draft.content
                ),
            ),
        ];

        let report = self.llm_provider.response(messages, &self.model).await?;
        Ok(report.trim().to_string())
    }

    async fn critique_final_report(
        &self,
        query: &str,
        plan: &ResearchPlan,
        report: &str,
    ) -> Result<String, LlmClientError> {
        let plan_text = plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let schema = StructuredOutputSchema {
            name: "final_report_critique".to_string(),
            description: Some("A critique of the initial final report.".to_string()),
            schema: json!({
                "type": "object",
                "properties": {
                    "critique": {
                        "type": "string",
                        "minLength": 1
                    }
                },
                "required": ["critique"],
                "additionalProperties": false
            }),
        };

        let messages = vec![
            Message::text(
                Role::System,
                "You are a final report critic. Review the report and identify weaknesses in coverage, coherence, redundancy, unsupported claims, or organization.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Critique the following final report.\n\nQuery: {query}\n\nResearch plan:\n{plan_text}\n\nReport:\n{report}\n\nRequirements:\n- Identify missing coverage relative to the research plan if any\n- Identify redundancy or poor organization if any\n- Identify unsupported or weakly justified claims if any\n- Be concise but specific\n- Return JSON only"
                ),
            ),
        ];

        let value = self
            .llm_provider
            .response_with_structure(messages, schema, &self.model)
            .await?;

        let critique = value
            .get("critique")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                LlmClientError::ResponseParse("failed to decode final report critique".to_string())
            })?
            .to_string();

        Ok(critique)
    }

    async fn revise_final_report(
        &self,
        query: &str,
        plan: &ResearchPlan,
        initial_report: &str,
        critique: &str,
    ) -> Result<String, LlmClientError> {
        let plan_text = plan
            .sections
            .iter()
            .enumerate()
            .map(|(index, section)| format!("{}. {}", index + 1, section))
            .collect::<Vec<_>>()
            .join("\n");

        let messages = vec![
            Message::text(
                Role::System,
                "You are a final report writing assistant. Revise the report using the critique and produce a better final report.",
            ),
            Message::text(
                Role::User,
                format!(
                    "Revise the following final report.\n\nQuery: {query}\n\nResearch plan:\n{plan_text}\n\nInitial final report:\n{initial_report}\n\nCritique:\n{critique}\n\nRequirements:\n- Write in Japanese\n- Preserve the useful content of the report while improving it\n- Improve coverage relative to the research plan if needed\n- Remove redundancy and improve organization\n- Resolve weak or unsupported statements where possible\n- If uncertainty remains, state it cautiously\n- Do not include citations or URLs\n- Return only the revised final report in Markdown"
                ),
            ),
        ];

        let report = self.llm_provider.response(messages, &self.model).await?;
        Ok(report.trim().to_string())
    }
}

fn format_search_documents(documents: &[SearchDocument]) -> String {
    documents
        .iter()
        .enumerate()
        .map(|(index, doc)| {
            format!(
                "[{}]\nTitle: {}\nURL: {}\nSnippet: {}",
                index + 1,
                doc.title,
                doc.url,
                doc.snippet
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_revision_history(revisions: &[DraftReport]) -> String {
    revisions
        .iter()
        .map(|draft| format!("Revision {}:\n{}", draft.revision, draft.content))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

fn format_answer_candidates(candidates: &[ResearchAnswer]) -> String {
    candidates
        .iter()
        .enumerate()
        .map(|(index, answer)| format!("[{}]\n{}", index + 1, answer.summary))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn print_progress_stage(message: &str) {
    println!("{message}");
}

fn print_iteration_summary(
    iteration: usize,
    question: &ResearchQuestion,
    answer: &ResearchAnswer,
    should_exit: bool,
) {
    println!("Iteration {iteration}");
    println!("- Focus: {}", question.focus);
    println!("- Question: {}", question.text);
    println!("- Finding: {}", shorten_for_progress(&answer.summary, 160));
    println!(
        "- Coverage: {}",
        if should_exit { "complete" } else { "continue" }
    );
}

fn shorten_for_progress(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let shortened = trimmed.chars().take(max_chars).collect::<String>();
    if trimmed.chars().count() > max_chars {
        format!("{shortened}...")
    } else {
        shortened
    }
}
