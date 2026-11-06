use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("network error: {0}")]
    Network(String),
    #[error("parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Station {
    #[serde(default)] pub stationuuid: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub url: String,
    #[serde(default)] pub url_resolved: String,
    #[serde(default)] pub homepage: String,
    #[serde(default)] pub favicon: String,
    #[serde(default)] pub tags: String,
    #[serde(default)] pub country: String,
    #[serde(default)] pub countrycode: String,
    #[serde(default)] pub language: String,
    #[serde(default)] pub votes: u32,
    #[serde(default)] pub bitrate: u32,
    #[serde(default)] pub codec: String,
    #[serde(default)] pub languagecodes: String,
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilters {
    pub query: String,
    pub country: Option<String>,
    pub countrycode: Option<String>,
    pub genre: Option<String>,
    pub language: Option<String>,
    pub codec: Option<String>,
    pub bitrate_min: Option<u32>,
    pub bitrate_max: Option<u32>,
    pub limit: usize,
}

impl SearchFilters {
    pub fn new() -> Self { Self { limit: 50, ..Default::default() }
    }
}

pub struct RadioBrowserClient {
    base_urls: Vec<String>,
    client: reqwest::Client,
}

impl RadioBrowserClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("ham-radio-app/0.1.0 (Rust)")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_urls: vec![
                "https://all.api.radio-browser.info".to_string(),
                "https://de1.api.radio-browser.info".to_string(),
                "https://nl1.api.radio-browser.info".to_string(),
                "https://fr1.api.radio-browser.info".to_string(),
                "https://at1.api.radio-browser.info".to_string(),
            ],
            client,
        }
    }

    pub async fn search(&self, query: &str, country: Option<&str>, tag: Option<&str>, limit: usize) -> Result<Vec<Station>, ApiError> {
        let mut f = SearchFilters { query: query.to_string(), limit, ..Default::default() };
        if let Some(c) = country {
            if c.len() == 2 { f.countrycode = Some(c.to_string()); } else { f.country = Some(c.to_string()); }
        }
        if let Some(t) = tag { f.genre = Some(t.to_string()); }
        self.search_filtered(f).await
    }

    pub async fn search_filtered(&self, filters: SearchFilters) -> Result<Vec<Station>, ApiError> {
        let limit = filters.limit.clamp(1, 100);
        let mut last_err = None;
        for base_url in &self.base_urls {
            let mut params: Vec<(String, String)> = vec![
                ("hidebroken".to_string(), "true".to_string()),
                ("order".to_string(), "votes".to_string()),
                ("reverse".to_string(), "true".to_string()),
                ("limit".to_string(), limit.to_string()),
            ];
            if !filters.query.trim().is_empty() { params.push(("name".to_string(), filters.query.trim().to_string())); }
            if let Some(c) = &filters.country { if !c.trim().is_empty() { params.push(("country".to_string(), c.trim().to_string())); } }
            if let Some(cc) = &filters.countrycode { if !cc.trim().is_empty() { params.push(("countrycode".to_string(), cc.trim().to_uppercase())); } }
            if let Some(g) = &filters.genre { if !g.trim().is_empty() { params.push(("tag".to_string(), g.trim().to_string())); } }
            if let Some(lang) = &filters.language { if !lang.trim().is_empty() { params.push(("language".to_string(), lang.trim().to_string())); } }
            if let Some(codec) = &filters.codec { if !codec.trim().is_empty() { params.push(("codec".to_string(), codec.trim().to_string())); } }
            if let Some(bmin) = filters.bitrate_min { if bmin > 0 { params.push(("bitrateMin".to_string(), bmin.to_string())); } }
            if let Some(bmax) = filters.bitrate_max { if bmax > 0 { params.push(("bitrateMax".to_string(), bmax.to_string())); } }

            let url = if params.len() == 4 {
                format!("{}/json/stations/topvote/{}", base_url, limit)
            } else {
                format!("{}/json/stations/search", base_url)
            };

            let result = if params.len() == 4 {
                self.client.get(&url).send().await
            } else {
                self.client.get(&url).query(&params).send().await
            };

            match result {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        last_err = Some(format!("HTTP {} from {}", resp.status(), base_url));
                        continue;
                    }
                    match resp.json::<Vec<Station>>().await {
                        Ok(stations) => return Ok(Self::filter_hls(stations)),
                        Err(e) => { last_err = Some(format!("parse {}: {}", base_url, e)); continue; }
                    }
                }
                Err(e) => { last_err = Some(format!("{} -> {}", base_url, e)); continue; }
            }
        }
        Err(ApiError::Network(format!("All mirrors failed. Last: {}", last_err.unwrap_or_default())))
    }

    // ONLY filter HLS m3u8 - DO NOT filter ODTU or HE-AAC, list everything
    // Player will handle decode failures gracefully without crashing
    fn filter_hls(stations: Vec<Station>) -> Vec<Station> {
        stations.into_iter()
            .filter(|s| {
                let url = if !s.url_resolved.is_empty() { &s.url_resolved } else { &s.url };
                let url_lower = url.to_lowercase();
                // Only block true HLS playlists, not AACP
                if url_lower.ends_with(".m3u8") || url_lower.contains(".m3u8?") {
                    return false;
                }
                true
            })
            .collect()
    }

    pub async fn stations_by_country(&self, countrycode: &str) -> Result<Vec<Station>, ApiError> {
        self.search("", Some(countrycode), None, 50).await
    }
    pub async fn stations_by_genre(&self, genre: &str) -> Result<Vec<Station>, ApiError> {
        self.search("", None, Some(genre), 50).await
    }
    pub async fn top_stations(&self, limit: usize) -> Result<Vec<Station>, ApiError> {
        for base_url in &self.base_urls {
            let url = format!("{}/json/stations/topvote/{}", base_url, limit.clamp(1, 100));
            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(stations) = resp.json::<Vec<Station>>().await {
                        return Ok(Self::filter_hls(stations));
                    }
                }
            }
        }
        Err(ApiError::Network("All mirrors failed".to_string()))
    }
}
