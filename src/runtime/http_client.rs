// FORGE HTTP client
// Outbound HTTP capabilities: web.fetch, web.post, search
// See issue #51

use reqwest::Client;
use std::time::Duration;

use crate::config::WebConfig;

pub struct ForgeHttpClient {
    client: Client,
}

impl ForgeHttpClient {
    pub fn new(config: Option<&WebConfig>) -> Self {
        let timeout = config.map(|c| c.timeout_or_default()).unwrap_or(30);
        let max_redirects = config.map(|c| c.max_redirects_or_default()).unwrap_or(10);
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout))
            .redirect(reqwest::redirect::Policy::limited(max_redirects))
            .user_agent("FORGE/1.0")
            .build()
            .expect("failed to build HTTP client");
        Self { client }
    }

    pub async fn fetch(&self, url: &str) -> Result<String, String> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| format_reqwest_error(e, url))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status.as_u16(), url));
        }

        response
            .text()
            .await
            .map_err(|e| format!("failed to read response body from {}: {}", url, e))
    }

    pub async fn post(&self, url: &str, body: &str) -> Result<String, String> {
        let response = self
            .client
            .post(url)
            .header("Content-Type", "text/plain")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format_reqwest_error(e, url))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status.as_u16(), url));
        }

        response
            .text()
            .await
            .map_err(|e| format!("failed to read response body from {}: {}", url, e))
    }
}

// ── Search ───────────────────────────────────────────────────────────────────

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

pub async fn search(
    client: &ForgeHttpClient,
    query: &str,
    config: Option<&WebConfig>,
) -> Result<Vec<SearchResult>, String> {
    let provider = config
        .and_then(|c| c.search_provider.as_deref())
        .unwrap_or("searxng");

    match provider {
        "searxng" => search_searxng(client, query, config).await,
        other => Err(format!(
            "unsupported search provider: {} (supported: searxng)",
            other
        )),
    }
}

async fn search_searxng(
    client: &ForgeHttpClient,
    query: &str,
    config: Option<&WebConfig>,
) -> Result<Vec<SearchResult>, String> {
    let base_url = config
        .and_then(|c| c.search_url.as_deref())
        .unwrap_or("http://localhost:8080");

    let url = format!(
        "{}/search?q={}&format=json",
        base_url.trim_end_matches('/'),
        urlencoding::encode(query)
    );

    let response = client
        .client
        .get(&url)
        .send()
        .await
        .map_err(|e| format_reqwest_error(e, &url))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("search failed: HTTP {}", status.as_u16()));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse search response: {}", e))?;

    let results = body["results"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|item| SearchResult {
                    title: item["title"].as_str().unwrap_or("").to_string(),
                    url: item["url"].as_str().unwrap_or("").to_string(),
                    snippet: item["content"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(results)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn format_reqwest_error(e: reqwest::Error, url: &str) -> String {
    if e.is_timeout() {
        format!("HTTP timeout: {}", url)
    } else if e.is_connect() {
        format!("connection failed: {}", url)
    } else {
        format!("HTTP error for {}: {}", url, e)
    }
}
