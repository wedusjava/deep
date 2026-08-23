use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use chrono::NaiveDate;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    clients::{FirecrawlClient, OpenAiClient},
    credentials::{FirecrawlProfile, LlmProfile},
    store::{ClaimRow, SourceRow, Store},
};

const MAX_AGENT_STEPS: usize = 48;
const MAX_TOOL_OUTPUT_CHARS: usize = 40_000;

const SYSTEM_PROMPT: &str = r#"You are the investigator inside Deep, an evidence-first research harness.

Your job is not to provide the most plausible answer. Your job is to establish only what the available evidence supports.

Admission gate:
- If the request does not require substantive external research, verification, source comparison, or investigation, call reject_request immediately.
- Never insult or judge the user for a rejected request.

Research rules:
1. Search results are discovery only. A search snippet is not evidence. Scrape the underlying page before recording evidence from it.
2. Prefer primary sources when they are appropriate to the claim. Source quality is claim-relative, not domain-relative.
3. Keep claims, evidence, and hypotheses separate. Never silently promote a hypothesis into a fact.
4. Every material supported or verified claim must be linked to evidence using the workspace tools.
5. Actively look for contradictory evidence and alternative explanations for material claims.
6. Treat duplicate or derivative sources as non-independent corroboration.
7. Use the minimum sufficient research. Stop when evidence is sufficient, genuinely conflicting, unavailable after reasonable search, a lead is exhausted, or new searches have diminishing returns.
8. Never make the conclusion stronger than the evidence.
9. It is valid to finish with insufficient evidence.
10. Do not reveal hidden chain-of-thought. Tool actions and concise operational rationales are enough.

Workflow:
- Discover candidate sources with search/map/crawl.
- Read candidate sources with scrape before treating their content as evidence.
- Record material claims.
- Record evidence excerpts only from scraped sources.
- Link evidence to claims and update claim states.
- Challenge important conclusions.
- Finish only through finish_report, or reject through reject_request.

Claim states are: VERIFIED, SUPPORTED, UNRESOLVED, CONFLICTING, DISPROVEN, INSUFFICIENT_EVIDENCE, DEAD_END.
Source classes are: PRIMARY, HIGH_QUALITY_SECONDARY, SECONDARY, WEAK.
Directness is DIRECT or INDIRECT.
Evidence relations are SUPPORTS or CONTRADICTS.

Write the human-facing conclusion and limitations in the user's language unless the user requests another language."#;

#[derive(Clone, Debug)]
pub enum ResearchEvent {
    Activity { kind: String, message: String },
    Claim {
        id: i64,
        statement: String,
        status: String,
    },
    Source {
        id: i64,
        title: String,
        url: String,
        source_class: String,
        duplicate_of: Option<i64>,
    },
    Finished { case_id: String, report: String },
    Failed { message: String },
}

pub async fn run_investigation(
    objective: String,
    llm_profile: LlmProfile,
    firecrawl_profile: FirecrawlProfile,
    db_path: PathBuf,
    tx: UnboundedSender<ResearchEvent>,
) {
    if let Err(error) = run(
        objective,
        llm_profile,
        firecrawl_profile,
        db_path,
        tx.clone(),
    )
    .await
    {
        let _ = tx.send(ResearchEvent::Failed {
            message: format!("{error:#}"),
        });
    }
}

async fn run(
    objective: String,
    llm_profile: LlmProfile,
    firecrawl_profile: FirecrawlProfile,
    db_path: PathBuf,
    tx: UnboundedSender<ResearchEvent>,
) -> Result<()> {
    let store = Store::open(&db_path)?;
    let case_id = store.create_case(&objective)?;
    let llm = OpenAiClient::new(llm_profile)?;
    let firecrawl = FirecrawlClient::new(firecrawl_profile)?;

    emit(
        &store,
        &tx,
        &case_id,
        "ADMISSION",
        "Evaluating whether the objective requires substantive research.",
    )?;

    let mut messages = vec![
        json!({"role": "system", "content": SYSTEM_PROMPT}),
        json!({"role": "user", "content": format!("Investigation objective:\n{objective}")}),
    ];
    let tools = tool_specs();
    let mut research_operations = 0usize;

    for step in 0..MAX_AGENT_STEPS {
        let response = llm.chat(&messages, &tools).await?;
        let message = response
            .pointer("/choices/0/message")
            .cloned()
            .ok_or_else(|| anyhow!("LLM response did not contain choices[0].message"))?;

        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        messages.push(message.clone());

        if tool_calls.is_empty() {
            let content = message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let reminder = if content.is_empty() {
                "Continue the investigation using tools. Finish only through finish_report or reject_request."
            } else {
                "Do not answer directly. Use the workspace and research tools, then finish only through finish_report or reject_request."
            };
            messages.push(json!({"role": "user", "content": reminder}));
            continue;
        }

        for call in tool_calls {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("tool call is missing id"))?;
            let function = call
                .get("function")
                .ok_or_else(|| anyhow!("tool call is missing function"))?;
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("tool call is missing function.name"))?;
            let arguments = parse_arguments(function.get("arguments"))?;

            if matches!(
                name,
                "search" | "scrape" | "map" | "crawl" | "crawl_status" | "interact"
            ) {
                research_operations += 1;
            }

            let outcome = execute_tool(
                name,
                &arguments,
                &store,
                &firecrawl,
                &case_id,
                &tx,
                research_operations,
            )
            .await;

            match outcome {
                Ok(ToolOutcome::Continue(content)) => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": truncate(&content, MAX_TOOL_OUTPUT_CHARS)
                    }));
                }
                Ok(ToolOutcome::Finish(report)) => {
                    let _ = tx.send(ResearchEvent::Finished {
                        case_id: case_id.clone(),
                        report,
                    });
                    return Ok(());
                }
                Err(error) => {
                    emit(
                        &store,
                        &tx,
                        &case_id,
                        "TOOL_ERROR",
                        &format!("{name}: {error:#}"),
                    )?;
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": format!("Tool error: {error:#}")
                    }));
                }
            }
        }

        if step + 1 == MAX_AGENT_STEPS {
            emit(
                &store,
                &tx,
                &case_id,
                "STOP",
                "Operational step guardrail reached before an epistemic stop condition.",
            )?;
        }
    }

    store.finish_case(&case_id, "guardrail")?;
    let report = build_report(
        &store,
        &case_id,
        "The investigation stopped at the operational step guardrail before a defensible conclusion was produced.",
        "This is an operational stop, not evidence that the objective is unanswerable.",
    )?;
    let _ = tx.send(ResearchEvent::Finished { case_id, report });
    Ok(())
}

enum ToolOutcome {
    Continue(String),
    Finish(String),
}

async fn execute_tool(
    name: &str,
    args: &Value,
    store: &Store,
    firecrawl: &FirecrawlClient,
    case_id: &str,
    tx: &UnboundedSender<ResearchEvent>,
    research_operations: usize,
) -> Result<ToolOutcome> {
    match name {
        "search" => {
            let query = required_str(args, "query")?;
            let limit = optional_u64(args, "limit").unwrap_or(8) as usize;
            emit(store, tx, case_id, "SEARCH", query)?;
            let value = firecrawl.search(query, limit).await?;
            emit(store, tx, case_id, "FOUND", "Search results received; snippets remain discovery-only.")?;
            Ok(ToolOutcome::Continue(value.to_string()))
        }
        "scrape" => {
            let url = required_str(args, "url")?;
            emit(store, tx, case_id, "SCRAPE", url)?;
            let value = firecrawl.scrape(url).await?;
            let markdown = extract_markdown(&value)
                .ok_or_else(|| anyhow!("Firecrawl scrape response did not contain markdown"))?;
            let title = extract_title(&value);
            let source_id = store.upsert_source(case_id, url, title.as_deref(), "unknown", markdown)?;
            let source = store
                .list_sources(case_id)?
                .into_iter()
                .find(|source| source.id == source_id)
                .ok_or_else(|| anyhow!("source was not persisted"))?;
            let _ = tx.send(source_event(&source));
            emit(
                store,
                tx,
                case_id,
                if source.duplicate_of.is_some() { "DUPLICATE" } else { "OPEN" },
                &format!("S{source_id} {url}"),
            )?;
            Ok(ToolOutcome::Continue(
                json!({
                    "source_id": source_id,
                    "url": url,
                    "title": title,
                    "scrape_id": extract_scrape_id(&value),
                    "markdown": truncate(markdown, MAX_TOOL_OUTPUT_CHARS)
                })
                .to_string(),
            ))
        }
        "map" => {
            let url = required_str(args, "url")?;
            let search = args.get("search").and_then(Value::as_str);
            let limit = optional_u64(args, "limit").unwrap_or(200) as usize;
            emit(store, tx, case_id, "MAP", url)?;
            let value = firecrawl.map(url, search, limit).await?;
            Ok(ToolOutcome::Continue(value.to_string()))
        }
        "crawl" => {
            let url = required_str(args, "url")?;
            let limit = optional_u64(args, "limit").unwrap_or(50) as usize;
            let max_depth = optional_u64(args, "max_depth").unwrap_or(2) as usize;
            emit(store, tx, case_id, "CRAWL", url)?;
            let value = firecrawl.crawl(url, limit, max_depth).await?;
            Ok(ToolOutcome::Continue(value.to_string()))
        }
        "crawl_status" => {
            let id = required_str(args, "id")?;
            emit(store, tx, case_id, "FOLLOW", &format!("crawl {id}"))?;
            let value = firecrawl.crawl_status(id).await?;
            Ok(ToolOutcome::Continue(value.to_string()))
        }
        "interact" => {
            let scrape_id = required_str(args, "scrape_id")?;
            let prompt = required_str(args, "prompt")?;
            emit(store, tx, case_id, "INTERACT", prompt)?;
            let value = firecrawl.interact(scrape_id, prompt).await?;
            Ok(ToolOutcome::Continue(value.to_string()))
        }
        "calculator" => {
            let expression = required_str(args, "expression")?;
            let value = meval::eval_str(expression).context("invalid arithmetic expression")?;
            emit(store, tx, case_id, "CALCULATE", expression)?;
            Ok(ToolOutcome::Continue(json!({"result": value}).to_string()))
        }
        "date_days_between" => {
            let start = NaiveDate::parse_from_str(required_str(args, "start")?, "%Y-%m-%d")?;
            let end = NaiveDate::parse_from_str(required_str(args, "end")?, "%Y-%m-%d")?;
            let days = end.signed_duration_since(start).num_days();
            emit(store, tx, case_id, "DATE_MATH", &format!("{start} → {end}"))?;
            Ok(ToolOutcome::Continue(json!({"days": days}).to_string()))
        }
        "record_claim" => {
            let statement = required_str(args, "statement")?;
            let id = store.record_claim(case_id, statement)?;
            emit(store, tx, case_id, "CLAIM", &format!("C{id} created"))?;
            let _ = tx.send(ResearchEvent::Claim {
                id,
                statement: statement.to_owned(),
                status: "UNRESOLVED".to_owned(),
            });
            Ok(ToolOutcome::Continue(json!({"claim_id": id}).to_string()))
        }
        "record_evidence" => {
            let source_url = required_str(args, "source_url")?;
            let source_id = store
                .source_id_by_url(case_id, source_url)?
                .ok_or_else(|| anyhow!("source must be scraped before evidence can be recorded"))?;
            let excerpt = required_str(args, "excerpt")?;
            let source_class = required_str(args, "source_class")?;
            let directness = required_str(args, "directness")?;
            let id = store.record_evidence(
                case_id,
                source_id,
                excerpt,
                source_class,
                directness,
            )?;
            emit(store, tx, case_id, "EVIDENCE", &format!("E{id} from S{source_id}"))?;
            Ok(ToolOutcome::Continue(json!({"evidence_id": id}).to_string()))
        }
        "link_evidence" => {
            let claim_id = required_i64(args, "claim_id")?;
            let evidence_id = required_i64(args, "evidence_id")?;
            let relation = required_str(args, "relation")?;
            store.link_evidence(claim_id, evidence_id, relation)?;
            emit(
                store,
                tx,
                case_id,
                "LINK",
                &format!("E{evidence_id} {relation} C{claim_id}"),
            )?;
            Ok(ToolOutcome::Continue("linked".to_owned()))
        }
        "set_claim_status" => {
            let claim_id = required_i64(args, "claim_id")?;
            let status = required_str(args, "status")?;
            let rationale = required_str(args, "rationale")?;
            store.set_claim_status(case_id, claim_id, status, rationale)?;
            let claim = find_claim(store, case_id, claim_id)?;
            emit(
                store,
                tx,
                case_id,
                "VERIFY",
                &format!("C{claim_id} → {status}"),
            )?;
            let _ = tx.send(ResearchEvent::Claim {
                id: claim.id,
                statement: claim.statement,
                status: claim.status,
            });
            Ok(ToolOutcome::Continue("claim status updated".to_owned()))
        }
        "finish_report" => {
            if research_operations == 0 {
                bail!("a research request cannot finish before at least one external research operation");
            }
            let conclusion = required_str(args, "conclusion")?;
            let limitations = required_str(args, "limitations")?;
            store.finish_case(case_id, "finished")?;
            emit(store, tx, case_id, "STOP", "Evidence-driven stop condition reached.")?;
            let report = build_report(store, case_id, conclusion, limitations)?;
            Ok(ToolOutcome::Finish(report))
        }
        "reject_request" => {
            let reason = required_str(args, "reason")?;
            store.finish_case(case_id, "rejected")?;
            emit(store, tx, case_id, "REJECT", reason)?;
            Ok(ToolOutcome::Finish(format!(
                "REJECTED\n\n{reason}\n\nThis request does not require substantive evidence-based investigation."
            )))
        }
        _ => bail!("unknown tool: {name}"),
    }
}

fn build_report(store: &Store, case_id: &str, conclusion: &str, limitations: &str) -> Result<String> {
    let claims = store.list_claims(case_id)?;
    let sources = store.list_sources(case_id)?;
    let mut output = String::new();
    output.push_str("RESULT\n\n");
    output.push_str(conclusion.trim());
    output.push_str("\n\nCLAIMS\n\n");

    if claims.is_empty() {
        output.push_str("No material claims were established.\n");
    } else {
        for claim in claims {
            let count = store.evidence_count_for_claim(claim.id)?;
            output.push_str(&format!(
                "C{}  {}  [evidence: {}]\n    {}\n",
                claim.id, claim.status, count, claim.statement
            ));
            if let Some(rationale) = claim.rationale {
                output.push_str(&format!("    rationale: {}\n", rationale.trim()));
            }
        }
    }

    output.push_str("\nSOURCES\n\n");
    if sources.is_empty() {
        output.push_str("No scraped source was accepted into the evidence workspace.\n");
    } else {
        for source in sources {
            let duplicate = source
                .duplicate_of
                .map(|id| format!(" duplicate-of:S{id}"))
                .unwrap_or_default();
            output.push_str(&format!(
                "S{}  {}{}\n    {}\n",
                source.id,
                source.source_class,
                duplicate,
                source.title.as_deref().unwrap_or(&source.url)
            ));
            output.push_str(&format!("    {}\n", source.url));
        }
    }

    output.push_str("\nLIMITATIONS\n\n");
    output.push_str(limitations.trim());
    output.push_str("\n\nCONCLUSION\n\n");
    output.push_str(conclusion.trim());
    output.push('\n');
    Ok(output)
}

fn tool_specs() -> Vec<Value> {
    vec![
        tool("search", "Discover candidate public web sources. Search results are not evidence.", json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":20}},"required":["query"]})),
        tool("scrape", "Read a candidate source. A source must be scraped before its content can become evidence.", json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]})),
        tool("map", "Map a website to discover relevant URLs.", json!({"type":"object","properties":{"url":{"type":"string"},"search":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":5000}},"required":["url"]})),
        tool("crawl", "Start a bounded crawl when relevant information is spread across a site.", json!({"type":"object","properties":{"url":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":1000},"max_depth":{"type":"integer","minimum":0,"maximum":10}},"required":["url"]})),
        tool("crawl_status", "Retrieve the status and current results of a Firecrawl crawl job.", json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})),
        tool("interact", "Interact with a previously scraped dynamic page using its scrape id.", json!({"type":"object","properties":{"scrape_id":{"type":"string"},"prompt":{"type":"string"}},"required":["scrape_id","prompt"]})),
        tool("calculator", "Evaluate deterministic arithmetic.", json!({"type":"object","properties":{"expression":{"type":"string"}},"required":["expression"]})),
        tool("date_days_between", "Calculate the signed number of days between two ISO dates.", json!({"type":"object","properties":{"start":{"type":"string","description":"YYYY-MM-DD"},"end":{"type":"string","description":"YYYY-MM-DD"}},"required":["start","end"]})),
        tool("record_claim", "Create a material claim in the research workspace. New claims begin UNRESOLVED.", json!({"type":"object","properties":{"statement":{"type":"string"}},"required":["statement"]})),
        tool("record_evidence", "Record an excerpt as evidence. source_url must already have been scraped in this case.", json!({"type":"object","properties":{"source_url":{"type":"string"},"excerpt":{"type":"string"},"source_class":{"type":"string","enum":["PRIMARY","HIGH_QUALITY_SECONDARY","SECONDARY","WEAK"]},"directness":{"type":"string","enum":["DIRECT","INDIRECT"]}},"required":["source_url","excerpt","source_class","directness"]})),
        tool("link_evidence", "Link evidence to a material claim.", json!({"type":"object","properties":{"claim_id":{"type":"integer"},"evidence_id":{"type":"integer"},"relation":{"type":"string","enum":["SUPPORTS","CONTRADICTS"]}},"required":["claim_id","evidence_id","relation"]})),
        tool("set_claim_status", "Update a claim's epistemic state. VERIFIED, SUPPORTED, and DISPROVEN require linked evidence.", json!({"type":"object","properties":{"claim_id":{"type":"integer"},"status":{"type":"string","enum":["VERIFIED","SUPPORTED","UNRESOLVED","CONFLICTING","DISPROVEN","INSUFFICIENT_EVIDENCE","DEAD_END"]},"rationale":{"type":"string"}},"required":["claim_id","status","rationale"]})),
        tool("finish_report", "Finish only when an evidence-driven stop condition is satisfied. The harness builds the report from structured workspace state.", json!({"type":"object","properties":{"conclusion":{"type":"string"},"limitations":{"type":"string"}},"required":["conclusion","limitations"]})),
        tool("reject_request", "Reject a request that does not require substantive evidence-based research.", json!({"type":"object","properties":{"reason":{"type":"string"}},"required":["reason"]})),
    ]
}

fn tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters
        }
    })
}

fn parse_arguments(value: Option<&Value>) -> Result<Value> {
    match value {
        Some(Value::String(raw)) if raw.trim().is_empty() => Ok(json!({})),
        Some(Value::String(raw)) => serde_json::from_str(raw).context("invalid tool arguments JSON"),
        Some(Value::Object(_)) => Ok(value.cloned().unwrap_or_else(|| json!({}))),
        Some(other) => Err(anyhow!("unexpected tool arguments: {other}")),
        None => Ok(json!({})),
    }
}

fn required_str<'a>(args: &'a Value, field: &str) -> Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("missing required string field: {field}"))
}

fn required_i64(args: &Value, field: &str) -> Result<i64> {
    args.get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("missing required integer field: {field}"))
}

fn optional_u64(args: &Value, field: &str) -> Option<u64> {
    args.get(field).and_then(Value::as_u64)
}

fn extract_markdown(value: &Value) -> Option<&str> {
    value.pointer("/data/markdown").and_then(Value::as_str)
}

fn extract_title(value: &Value) -> Option<String> {
    value
        .pointer("/data/metadata/title")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn extract_scrape_id(value: &Value) -> Option<String> {
    value
        .pointer("/data/metadata/scrapeId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn find_claim(store: &Store, case_id: &str, claim_id: i64) -> Result<ClaimRow> {
    store
        .list_claims(case_id)?
        .into_iter()
        .find(|claim| claim.id == claim_id)
        .ok_or_else(|| anyhow!("claim C{claim_id} was not found"))
}

fn source_event(source: &SourceRow) -> ResearchEvent {
    ResearchEvent::Source {
        id: source.id,
        title: source
            .title
            .clone()
            .unwrap_or_else(|| source.url.clone()),
        url: source.url.clone(),
        source_class: source.source_class.clone(),
        duplicate_of: source.duplicate_of,
    }
}

fn emit(
    store: &Store,
    tx: &UnboundedSender<ResearchEvent>,
    case_id: &str,
    kind: &str,
    message: &str,
) -> Result<()> {
    store.record_event(case_id, kind, message)?;
    let _ = tx.send(ResearchEvent::Activity {
        kind: kind.to_owned(),
        message: message.to_owned(),
    });
    Ok(())
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_is_english_and_contains_invariant() {
        assert!(SYSTEM_PROMPT.contains("Never make the conclusion stronger than the evidence"));
        assert!(!SYSTEM_PROMPT.contains("bukti"));
    }

    #[test]
    fn parses_string_tool_arguments() {
        let args = parse_arguments(Some(&Value::String("{\"query\":\"x\"}".into()))).unwrap();
        assert_eq!(args["query"], "x");
    }
}
