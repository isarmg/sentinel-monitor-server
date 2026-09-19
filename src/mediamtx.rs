use crate::error::{AppError, Result};
use chrono::{DateTime, Utc};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Clone)]
pub struct MediaMtxClient {
    client: Client,
    stream_client: Client,
    api_url: String,
    playback_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListPage<T> {
    item_count: usize,
    page_count: usize,
    items: Vec<T>,
}

#[derive(Deserialize)]
struct PathItem {
    name: String,
    ready: bool,
    readers: Vec<Value>,
    tracks: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathConfigItem {
    name: String,
    source: String,
    source_on_demand: bool,
    record: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PathSnapshot {
    pub ready: bool,
    pub readers: usize,
    pub tracks: usize,
}

#[derive(Clone, Debug, Default)]
pub struct PathConfigSnapshot {
    pub source_digest: Option<[u8; 32]>,
    pub source_on_demand: bool,
    pub record: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingSpan {
    pub start: String,
    pub duration: f64,
    #[serde(default)]
    pub url: Option<String>,
}

impl MediaMtxClient {
    pub fn new(
        client: Client,
        stream_client: Client,
        api_url: String,
        playback_url: String,
    ) -> Self {
        Self {
            client,
            stream_client,
            api_url,
            playback_url,
        }
    }

    pub async fn health(&self) -> bool {
        self.client
            .get(format!("{}/v3/info", self.api_url))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    pub async fn upsert_path(
        &self,
        path: &str,
        source: &str,
        source_on_demand: bool,
        record: bool,
    ) -> Result<()> {
        let get_url = format!("{}/v3/config/paths/get/{path}", self.api_url);
        let status = self
            .client
            .get(&get_url)
            .send()
            .await
            .map_err(|_| AppError::UpstreamUnknown("path lookup did not complete".into()))?
            .status();
        let exists = if status.is_success() {
            true
        } else if status == reqwest::StatusCode::NOT_FOUND {
            false
        } else {
            return Err(AppError::Upstream(format!("path lookup returned {status}")));
        };

        let payload = json!({
            "source": source,
            "sourceOnDemand": source_on_demand,
            "rtspTransport": "tcp",
            "record": record
        });
        let url = if exists {
            format!("{}/v3/config/paths/patch/{path}", self.api_url)
        } else {
            format!("{}/v3/config/paths/add/{path}", self.api_url)
        };
        let request = if exists {
            self.client.patch(url)
        } else {
            self.client.post(url)
        };
        let response = request
            .json(&payload)
            .send()
            .await
            .map_err(|_| AppError::UpstreamUnknown("path update did not complete".into()))?;
        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Upstream(format!("path update returned {status}")));
        }
        Ok(())
    }

    pub async fn delete_path(&self, path: &str) -> Result<()> {
        let response = self
            .client
            .delete(format!("{}/v3/config/paths/delete/{path}", self.api_url))
            .send()
            .await
            .map_err(|_| AppError::UpstreamUnknown("path deletion did not complete".into()))?;
        if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(AppError::Upstream(format!(
            "path deletion returned {}",
            response.status()
        )))
    }

    pub async fn paths(&self) -> Result<HashMap<String, PathSnapshot>> {
        let items: Vec<PathItem> = self.paginated("v3/paths/list", "path inventory").await?;
        let mut paths = HashMap::new();
        for item in items {
            if item.name.is_empty() || paths.contains_key(&item.name) {
                return Err(AppError::Upstream(
                    "path inventory contained an invalid or duplicate name".into(),
                ));
            }
            paths.insert(
                item.name,
                PathSnapshot {
                    ready: item.ready,
                    readers: item.readers.len(),
                    tracks: item.tracks.len(),
                },
            );
        }
        Ok(paths)
    }

    pub async fn path_configs(&self) -> Result<HashMap<String, PathConfigSnapshot>> {
        let items: Vec<PathConfigItem> = self
            .paginated("v3/config/paths/list", "path configuration inventory")
            .await?;
        let mut paths = HashMap::new();
        for item in items {
            if item.name.is_empty() || paths.contains_key(&item.name) {
                return Err(AppError::Upstream(
                    "path configuration inventory contained an invalid or duplicate name".into(),
                ));
            }
            let source_digest = Some(source_digest(&item.source));
            paths.insert(
                item.name,
                PathConfigSnapshot {
                    source_digest,
                    source_on_demand: item.source_on_demand,
                    record: item.record,
                },
            );
        }
        Ok(paths)
    }

    async fn paginated<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        label: &str,
    ) -> Result<Vec<T>> {
        const PAGE_SIZE: usize = 100;
        const MAX_PAGES: usize = 10_000;
        let mut page = 0_usize;
        let mut expected = None;
        let mut items = Vec::new();
        loop {
            let response = self
                .client
                .get(format!("{}/{path}", self.api_url))
                .query(&[("page", page), ("itemsPerPage", PAGE_SIZE)])
                .send()
                .await
                .map_err(|_| AppError::UpstreamUnknown(format!("{label} did not complete")))?;
            if !response.status().is_success() {
                return Err(AppError::Upstream(format!(
                    "{label} returned {}",
                    response.status()
                )));
            }
            let current: ListPage<T> = response
                .json()
                .await
                .map_err(|_| AppError::Upstream(format!("{label} response was invalid")))?;
            if current.page_count > MAX_PAGES
                || current.items.len() > PAGE_SIZE
                || expected.is_some_and(|value| value != (current.item_count, current.page_count))
            {
                return Err(AppError::Upstream(format!(
                    "{label} pagination was inconsistent"
                )));
            }
            expected = Some((current.item_count, current.page_count));
            items.extend(current.items);
            if page + 1 >= current.page_count {
                break;
            }
            page += 1;
        }
        if expected.is_none_or(|(count, _)| count != items.len()) {
            return Err(AppError::Upstream(format!(
                "{label} item count was inconsistent"
            )));
        }
        Ok(items)
    }

    pub async fn recordings(
        &self,
        path: &str,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        token: &str,
    ) -> Result<Vec<RecordingSpan>> {
        let mut query = vec![("path", path.to_string())];
        if let Some(start) = start {
            query.push(("start", start.to_rfc3339()));
        }
        if let Some(end) = end {
            query.push(("end", end.to_rfc3339()));
        }
        let response = self
            .client
            .get(format!("{}/list", self.playback_url))
            .bearer_auth(token)
            .query(&query)
            .send()
            .await
            .map_err(|_| AppError::Upstream("recording list request failed".into()))?;
        if !response.status().is_success() {
            return Err(AppError::Upstream(format!(
                "recording list returned {}",
                response.status()
            )));
        }
        response
            .json()
            .await
            .map_err(|_| AppError::Upstream("recording list response was invalid".into()))
    }

    pub async fn recording_stream(
        &self,
        path: &str,
        start: DateTime<Utc>,
        duration: f64,
        format: &str,
        token: &str,
        range: Option<&str>,
    ) -> Result<Response> {
        let mut request = self
            .stream_client
            .get(format!("{}/get", self.playback_url))
            .bearer_auth(token)
            .query(&[
                ("path", path.to_string()),
                ("start", start.to_rfc3339()),
                ("duration", duration.to_string()),
                ("format", format.to_string()),
            ]);
        if let Some(range) = range {
            request = request.header(reqwest::header::RANGE, range);
        }
        request
            .send()
            .await
            .map_err(|_| AppError::Upstream("recording stream request failed".into()))
    }
}

pub fn source_digest(source: &str) -> [u8; 32] {
    Sha256::digest(source.as_bytes()).into()
}
