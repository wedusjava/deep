use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::{Value, json};

use crate::credentials::{FirecrawlProfile, LlmProfile};

#[derive(Clone)]
pub struct OpenAiClient {
    http: Client,
    profile: LlmProfile,
}

impl OpenAiClient {
    pub fn new(profile: LlmProfile) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!("deep/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to initialize HTTP client")?;
        Ok(Self { http, profile })
    }

    pub async fn chat(&self, messages: &[Value], tools: &[Value]) -> Result<Value> {
        let endpoint = join_endpoint(&self.profile.base_url, "chat/completions");
        let response = self
            .http
            .post(endpoint)
            .bearer_auth(&self.profile.api_key)
            .json(&json!({
                "model": self.profile.model,
                "messages": messages,
                "tools": tools,
                "tool_choice": "auto",
                "temperature": 0.1
            }))
            .send()
            .await
            .context("LLM request failed")?;

        decode_json(response, "LLM").await
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
        let response = self
            .http
            .post(join_endpoint(&self.profile.base_url, path))
            .bearer_auth(&self.profile.api_key)
            .json(&body)
            .send()
            .await
            .context("Firecrawl request failed")?;
        decode_json(response, "Firecrawl").await
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .get(join_endpoint(&self.profile.base_url, path))
            .bearer_auth(&self.profile.api_key)
            .send()
            .await
            .context("Firecrawl request failed")?;
        decode_json(response, "Firecrawl").await
    }
}

async fn decode_json(response: reqwest::Response, service: &str) -> Result<Value> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed reading {service} response"))?;

    if !status.is_success() {
        let message = String::from_utf8_lossy(&bytes);
        bail!("{service} returned HTTP {status}: {}", truncate(&message, 2_000));
    }

    serde_json::from_slice(&bytes)
        .with_context(|| format!("{service} returned an invalid JSON response"))
}

fn join_endpoint(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
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
}
