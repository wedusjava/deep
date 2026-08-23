use std::{fs, path::{Path, PathBuf}};

use anyhow::{Result, anyhow};
use tokio::{
    sync::mpsc::{self, UnboundedSender},
    task::JoinSet,
};
use uuid::Uuid;

use crate::{
    agent::{ResearchEvent, run_investigation},
    clients::OpenAiClient,
    credentials::{FirecrawlProfile, LlmProfile},
};

const ID_NAMESPACE: i64 = 1_000_000;

const COORDINATOR_PROMPT: &str = r#"You are the coordinator of an OSINT analyst swarm.

Scope is strictly public-source OSINT. Synthesize only from the worker reports supplied to you. Never invent a source, observation, identity, relationship, date, or certainty level that is not supported by those reports.

Rules:
- Treat worker reports as independent analyst work products, not automatically as independent evidence.
- Prefer primary-source findings and explicitly note when corroboration is derivative or weak.
- Preserve contradictions and uncertainty instead of averaging them away.
- Separate established findings, supported findings, unresolved/conflicting items, and collection gaps.
- Keep source references and URLs present in the worker reports whenever useful.
- Do not provide instructions for intrusion, credential theft, bypassing access controls, stalking, or obtaining non-public data.
- If the workers do not establish the objective, say that evidence is insufficient.
- Write the final report in the user's language where practical.

Use this structure:
# OSINT Swarm Assessment
## Executive finding
## Established and supported findings
## Contradictions and unresolved items
## Source/evidence notes
## Collection gaps and limitations
"#;

#[derive(Clone, Copy)]
struct WorkerSpec {
    name: &'static str,
    slug: &'static str,
    mission: &'static str,
    directive: &'static str,
}

const WORKERS: [WorkerSpec; 3] = [
    WorkerSpec {
        name: "SCOUT",
        slug: "scout",
        mission: "Source discovery & entity mapping",
        directive: "Map the public information surface broadly. Discover primary sources, official records, relevant entities, aliases, domains, and high-value leads. Do not over-verify every lead; optimize for coverage while preserving evidence discipline.",
    },
    WorkerSpec {
        name: "VERIFIER",
        slug: "verifier",
        mission: "Claim verification & contradiction hunting",
        directive: "Identify the material claims implicit in the objective and test them. Prefer primary sources, seek contradictory evidence and alternative explanations, classify source quality, and avoid promoting search snippets into evidence.",
    },
    WorkerSpec {
        name: "TIMELINE",
        slug: "timeline",
        mission: "Chronology & cross-source correlation",
        directive: "Build and verify the relevant chronology from public sources. Correlate dates, actors, publications, changes, and sequences of events. Flag timestamp ambiguity, copied reporting, and causal claims that chronology alone cannot establish.",
    },
];

struct WorkerResult {
    name: &'static str,
    mission: &'static str,
    report: Option<String>,
    error: Option<String>,
}

pub async fn run_swarm_investigation(
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
            message: format!("OSINT swarm failed: {error:#}"),
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
    let swarm_id = Uuid::new_v4().to_string();
    let _ = tx.send(ResearchEvent::Activity {
        kind: "SWARM_START".into(),
        message: format!(
            "OSINT swarm {swarm_id} launching {} specialist workers",
            WORKERS.len()
        ),
    });

    let mut join_set = JoinSet::new();
    for (index, spec) in WORKERS.iter().copied().enumerate() {
        let worker_db = worker_db_path(&db_path, &swarm_id, spec.slug);
        let worker_objective = scoped_objective(&objective, spec);
        let llm = llm_profile.clone();
        let firecrawl = firecrawl_profile.clone();
        let output_tx = tx.clone();
        let namespace = (index as i64 + 1) * ID_NAMESPACE;

        let _ = tx.send(ResearchEvent::Activity {
            kind: "AGENT_START".into(),
            message: format!("{}|{}", spec.name, spec.mission),
        });

        join_set.spawn(async move {
            run_worker(
                spec,
                namespace,
                worker_objective,
                llm,
                firecrawl,
                worker_db,
                output_tx,
            )
            .await
        });
    }

    let mut results = Vec::new();
    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok(result) => {
                if let Some(error) = &result.error {
                    let _ = tx.send(ResearchEvent::Activity {
                        kind: "AGENT_ERROR".into(),
                        message: format!("{}|{}", result.name, error),
                    });
                } else {
                    let _ = tx.send(ResearchEvent::Activity {
                        kind: "AGENT_DONE".into(),
                        message: format!("{}|Completed specialist pass", result.name),
                    });
                }
                results.push(result);
            }
            Err(error) => {
                let _ = tx.send(ResearchEvent::Activity {
                    kind: "AGENT_ERROR".into(),
                    message: format!("WORKER|join failure: {error}"),
                });
            }
        }
    }

    let successful = results
        .iter()
        .filter(|result| result.report.is_some())
        .count();
    if successful == 0 {
        return Err(anyhow!("all specialist workers failed before producing a report"));
    }

    let _ = tx.send(ResearchEvent::Activity {
        kind: "SYNTHESIZE".into(),
        message: format!(
            "COORDINATOR|Synthesizing {successful}/{} successful worker reports",
            WORKERS.len()
        ),
    });

    let payload = coordinator_input(&objective, &results);
    let llm = OpenAiClient::new(llm_profile)?;
    let report = llm.complete(COORDINATOR_PROMPT, &payload).await?;

    let _ = tx.send(ResearchEvent::Finished {
        case_id: format!("osint-swarm-{swarm_id}"),
        report,
    });
    Ok(())
}

async fn run_worker(
    spec: WorkerSpec,
    namespace: i64,
    objective: String,
    llm_profile: LlmProfile,
    firecrawl_profile: FirecrawlProfile,
    db_path: PathBuf,
    tx: UnboundedSender<ResearchEvent>,
) -> WorkerResult {
    let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();
    let runner = tokio::spawn(run_investigation(
        objective,
        llm_profile,
        firecrawl_profile,
        db_path.clone(),
        worker_tx,
    ));

    let mut report = None;
    let mut error = None;

    while let Some(event) = worker_rx.recv().await {
        match event {
            ResearchEvent::Finished { report: value, .. } => {
                report = Some(value);
                break;
            }
            ResearchEvent::Failed { message } => {
                error = Some(message);
                break;
            }
            other => forward_worker_event(&tx, spec.name, namespace, other),
        }
    }

    if let Err(join_error) = runner.await
        && error.is_none()
    {
        error = Some(format!("worker task failed: {join_error}"));
    }
    if report.is_none() && error.is_none() {
        error = Some("worker stopped without a final report".into());
    }

    cleanup_workspace(&db_path);
    WorkerResult {
        name: spec.name,
        mission: spec.mission,
        report,
        error,
    }
}

fn forward_worker_event(
    tx: &UnboundedSender<ResearchEvent>,
    agent: &str,
    namespace: i64,
    event: ResearchEvent,
) {
    let event = match event {
        ResearchEvent::Activity { kind, message } => ResearchEvent::Activity {
            kind,
            message: format!("[{agent}] {message}"),
        },
        ResearchEvent::Claim {
            id,
            statement,
            status,
        } => ResearchEvent::Claim {
            id: namespace + id,
            statement: format!("[{agent}] {statement}"),
            status,
        },
        ResearchEvent::Lead {
            id,
            description,
            status,
        } => ResearchEvent::Lead {
            id: namespace + id,
            description: format!("[{agent}] {description}"),
            status,
        },
        ResearchEvent::Source {
            id,
            title,
            url,
            source_class,
            duplicate_of,
        } => ResearchEvent::Source {
            id: namespace + id,
            title: if title.trim().is_empty() {
                format!("[{agent}] {url}")
            } else {
                format!("[{agent}] {title}")
            },
            url,
            source_class,
            duplicate_of: duplicate_of.map(|source_id| namespace + source_id),
        },
        ResearchEvent::Finished { .. } | ResearchEvent::Failed { .. } => return,
    };
    let _ = tx.send(event);
}

fn scoped_objective(objective: &str, spec: WorkerSpec) -> String {
    format!(
        "PUBLIC-SOURCE OSINT ONLY. Use only lawfully accessible public information and the provided research tools. Do not seek private credentials, bypass access controls, intrude into systems, stalk private individuals, or obtain non-public data. Reject any part of the objective that would require those actions.\n\nSpecialist role: {} — {}\nRole directive: {}\n\nOriginal OSINT objective:\n{}",
        spec.name, spec.mission, spec.directive, objective
    )
}

fn coordinator_input(objective: &str, results: &[WorkerResult]) -> String {
    let mut output = format!("Original objective:\n{objective}\n\nWorker reports:\n");
    for result in results {
        output.push_str(&format!(
            "\n===== {} / {} =====\n",
            result.name, result.mission
        ));
        match (&result.report, &result.error) {
            (Some(report), _) => output.push_str(report),
            (None, Some(error)) => output.push_str(&format!("Worker failed: {error}")),
            (None, None) => output.push_str("Worker produced no report."),
        }
        output.push('\n');
    }
    output
}

fn worker_db_path(base: &Path, swarm_id: &str, slug: &str) -> PathBuf {
    let parent = base.parent().unwrap_or_else(|| Path::new("."));
    let stem = base
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("workspace");
    parent.join(format!("{stem}.swarm-{swarm_id}-{slug}.db"))
}

fn cleanup_workspace(path: &Path) {
    let _ = fs::remove_file(path);
    if let Some(value) = path.to_str() {
        let _ = fs::remove_file(format!("{value}-wal"));
        let _ = fs::remove_file(format!("{value}-shm"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_paths_are_isolated() {
        let base = Path::new("/tmp/deep/workspace.db");
        let scout = worker_db_path(base, "abc", "scout");
        let verifier = worker_db_path(base, "abc", "verifier");
        assert_ne!(scout, verifier);
        assert!(scout.to_string_lossy().contains("swarm-abc-scout"));
    }

    #[test]
    fn scoped_objective_is_explicitly_public_source_only() {
        let scoped = scoped_objective("verify example", WORKERS[0]);
        assert!(scoped.contains("PUBLIC-SOURCE OSINT ONLY"));
        assert!(scoped.contains("Original OSINT objective"));
    }
}
