use anyhow::Result;
use serde::{Deserialize, Serialize};

const BRAVE_BASE_URL: &str = "https://api.search.brave.com/res/v1";

pub struct BraveSearchState {
    api_key: Option<String>,
    client: reqwest::Client,
}

impl BraveSearchState {
    pub fn from_api_key(api_key: Option<String>) -> Self {
        if api_key.is_some() {
            tracing::info!("Brave Search tools enabled");
        } else {
            tracing::debug!("Brave Search API key not configured");
        }
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }

    fn get_api_key(&self) -> Result<&str> {
        self.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Brave Search API key not configured. Set api_keys.brave in settings."
            )
        })
    }

    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    pub async fn web_search(
        &self,
        query: &str,
        count: Option<u32>,
        freshness: Option<&str>,
    ) -> Result<BraveSearchResults> {
        let api_key = self.get_api_key()?;
        let encoded_q: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("q", query)
            .finish();
        let mut url = format!("{}/web/search?{}", BRAVE_BASE_URL, encoded_q);

        if let Some(c) = count {
            url.push_str(&format!("&count={}", c));
        }
        if let Some(f) = freshness {
            url.push_str(&format!("&freshness={}", f));
        }

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .header("Accept-Encoding", "gzip")
            .header("X-Subscription-Token", api_key)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Brave Search request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Brave Search API returned {}: {}",
                status,
                error_text
            ));
        }

        let body: BraveApiResponse = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse Brave response: {}", e))?;

        let mut results = Vec::new();
        if let Some(web) = body.web {
            for r in web.results {
                results.push(BraveSearchResult {
                    title: r.title,
                    url: r.url,
                    description: r.description.unwrap_or_default(),
                    age: r.age,
                });
            }
        }

        Ok(BraveSearchResults {
            query: query.to_string(),
            results,
            infobox: body.infobox.map(|ib| BraveInfobox {
                title: ib.results.first().map(|r| r.title.clone()).unwrap_or_default(),
                description: ib
                    .results
                    .first()
                    .and_then(|r| r.long_desc.clone())
                    .unwrap_or_default(),
                url: ib
                    .results
                    .first()
                    .map(|r| r.url.clone())
                    .unwrap_or_default(),
            }),
        })
    }
}

impl Default for BraveSearchState {
    fn default() -> Self {
        Self::from_api_key(None)
    }
}

#[derive(Debug, Deserialize)]
struct BraveApiResponse {
    web: Option<BraveWebResults>,
    infobox: Option<BraveInfoboxResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    results: Vec<BraveWebResult>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResult {
    title: String,
    url: String,
    description: Option<String>,
    age: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BraveInfoboxResults {
    results: Vec<BraveInfoboxResult>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BraveInfoboxResult {
    title: String,
    url: String,
    description: Option<String>,
    long_desc: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BraveSearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub age: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BraveInfobox {
    pub title: String,
    pub description: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct BraveSearchResults {
    pub query: String,
    pub results: Vec<BraveSearchResult>,
    pub infobox: Option<BraveInfobox>,
}
