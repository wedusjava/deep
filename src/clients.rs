use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::{Client, Method, StatusCode, header::RETRY_AFTER};
use serde_json::{Value, json};
use tokio::time::sleep;

use crate::credentials::{FirecrawlProfile, LlmProfile};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const LLM_TIMEOUT: Duration = Duration::from_secs(180);
const FIRECRAWL_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ERROR_CHARS: usize = 420;
const MAX_REQUEST_ATTEMPTS: usize = 4;
const BASE_RETRY_DELAY: Duration = Duration::from_millis(400);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(8);

#[derive(Clone)]
pub struct OpenAiClient {
    http: Client,
    profile: LlmProfile,
}

impl OpenAiClient {
    pub fn new(profile: LlmProfile) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!("deep/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(LLM_TIMEOUT)
            .build()
            .context("failed to initialize HTTP client")?;
        Ok(Self { http, profile })
    }

    pub async fn chat(&self, messages: &[Value], tools: &[Value]) -> Result<Value> {
        let body = json!({
            "model": self.profile.model,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
            "temperature": 0.1
        });
        self.post_chat(body).await
    }

    pub async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let body = json!({
            "model": self.profile.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "temperature": 0.1
        });
        let response = self.post_chat(body).await?;
        response
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("LLM response did not contain choices[0].message.content"))
    }

    async fn post_chat(&self, body: Value) -> Result<Value> {
        request_json_with_retry(
            &self.http,
            Method::POST,
            join_endpoint(&self.profile.base_url, "chat/completions"),
            &self.profile.api_key,
            Some(&body),
            "LLM",
        )
        .await
    }
}

#[derive(Clone)]
pub struct FirecrawlClient {
    http: Client,
    profile: FirecrawlProfile,
}

impl FirecrawlClient {
    pub fn new(profile: FirecrawlProfile) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!("deep/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(FIRECRAWL_TIMEOUT)
            .build()
            .context("failed to initialize HTTP client")?;
        Ok(Self { http, profile })
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Value> {
        self.post(
            "v2/search",
            json!({
                "query": query,
                "limit": limit.clamp(1, 20),
                "sources": ["web"]
            }),
        )
        .await
    }

    pub async fn scrape(&self, url: &str) -> Result<Value> {
        self.post(
            "v2/scrape",
            json!({
                "url": url,
                "formats": ["markdown"],
                "onlyMainContent": true,
                "removeBase64Images": true,
                "blockAds": true
            }),
        )
        .await
    }

    pub async fn map(&self, url: &str, search: Option<&str>, limit: usize) -> Result<Value> {
        self.post(
            "v2/map",
            json!({
                "url": url,
                "search": search,
                "sitemap": "include",
                "includeSubdomains": true,
                "ignoreQueryParameters": true,
                "limit": limit.clamp(1, 5000)
            }),
        )
        .await
    }

    pub async fn crawl(&self, url: &str, limit: usize, max_depth: usize) -> Result<Value> {
        self.post(
            "v2/crawl",
            json!({
                "url": url,
                "limit": limit.clamp(1, 1000),
                "maxDiscoveryDepth": max_depth.clamp(0, 10),
                "sitemap": "include",
                "ignoreQueryParameters": true,
                "scrapeOptions": {
                    "formats": ["markdown"],
                    "onlyMainContent": true,
                    "removeBase64Images": true,
                    "blockAds": true
                }
            }),
        )
        .await
    }

    pub async fn crawl_status(&self, id: &str) -> Result<Value> {
        self.get(&format!("v2/crawl/{id}")).await
    }

    pub async fn interact(&self, scrape_id: &str, prompt: &str) -> Result<Value> {
        self.post(
            &format!("v2/scrape/{scrape_id}/interact"),
            json!({
                "prompt": prompt,
                "timeout": 60,
                "origin": "deep"
            }),
        )
        .await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        request_json_with_retry(
            &self.http,
            Method::POST,
            join_endpoint(&self.profile.base_url, path),
            &self.profile.api_key,
            Some(&body),
            "Firecrawl",
        )
        .await
    }

    async fn get(&self, path: &str) -> Result<Value> {
        request_json_with_retry(
            &self.http,
            Method::GET,
            join_endpoint(&self.profile.base_url, path),
            &self.profile.api_key,
            None,
            "Firecrawl",
        )
        .await
    }
}

async fn request_json_with_retry(
    client: &Client,
    method: Method,
    url: String,
    bearer_token: &str,
    body: Option<&Value>,
    service: &str,
) -> Result<Value> {
    for attempt in 1..=MAX_REQUEST_ATTEMPTS {
        let mut request = client
            .request(method.clone(), &url)
            .bearer_auth(bearer_token);
        if let Some(body) = body {
            request = request.json(body);
        }

        match request.send().await {
            Ok(response) => {
                if is_retryable_status(response.status()) && attempt < MAX_REQUEST_ATTEMPTS {
                    let delay = retry_after(&response).unwrap_or_else(|| retry_delay(attempt));
                    sleep(delay).await;
                    continue;
                }
                return decode_json(response, service).await;
            }
            Err(error) if is_retryable_error(&error) && attempt < MAX_REQUEST_ATTEMPTS => {
                sleep(retry_delay(attempt)).await;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("{service} request failed"));
            }
        }
    }

    Err(anyhow!(
        "{service} request exhausted {MAX_REQUEST_ATTEMPTS} attempts"
    ))
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.as_u16() == 425
        || status.is_server_error()
}

fn is_retryable_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let seconds = response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(seconds).min(MAX_RETRY_DELAY))
}

fn retry_delay(attempt: usize) -> Duration {
    let shift = attempt.saturating_sub(1).min(8) as u32;
    let multiplier = 1u32 << shift;
    BASE_RETRY_DELAY
        .saturating_mul(multiplier)
        .min(MAX_RETRY_DELAY)
}

async fn decode_json(response: reqwest::Response, service: &str) -> Result<Value> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed reading {service} response"))?;

    if !status.is_success() {
        bail!(
            "{service} returned HTTP {status}: {}",
            response_error_message(&bytes)
        );
    }

    serde_json::from_slice(&bytes)
        .with_context(|| format!("{service} returned an invalid JSON response"))
}

fn response_error_message(bytes: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        for pointer in [
            "/error",
            "/message",
            "/detail",
            "/data/error",
            "/data/message",
        ] {
            if let Some(message) = value.pointer(pointer).and_then(Value::as_str)
                && !message.trim().is_empty()
            {
                return truncate(message.trim(), MAX_ERROR_CHARS);
            }
        }
        return truncate(&value.to_string(), MAX_ERROR_CHARS);
    }

    truncate(String::from_utf8_lossy(bytes).trim(), MAX_ERROR_CHARS)
}

fn join_endpoint(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut output = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_joining_preserves_v1_base() {
        assert_eq!(
            join_endpoint("https://api.example.test/v1/", "/chat/completions"),
            "https://api.example.test/v1/chat/completions"
        );
    }

    #[test]
    fn extracts_structured_error_message() {
        let body = br#"{"success":false,"error":"blocked by upstream"}"#;
        assert_eq!(response_error_message(body), "blocked by upstream");
    }

    #[test]
    fn bounds_unstructured_error_message() {
        let body = vec![b'x'; MAX_ERROR_CHARS + 100];
        let message = response_error_message(&body);
        assert!(message.chars().count() <= MAX_ERROR_CHARS);
        assert!(message.ends_with('…'));
    }

    #[test]
    fn retries_only_transient_statuses() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
    }

    #[test]
    fn backoff_is_bounded() {
        assert_eq!(retry_delay(1), BASE_RETRY_DELAY);
        assert!(retry_delay(10) <= MAX_RETRY_DELAY);
    }
}
