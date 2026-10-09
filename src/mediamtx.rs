use crate::error::{AppError, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const MAX_PLAYBACK_SECONDS: f64 = 21_600.0;
const MAX_RECORDING_WINDOWS: usize = 10_000;

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
        // MediaMTX 1.20 returns 404 when the selected path/time range has no
        // recording segments. That is an empty search result, not a failed API.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !response.status().is_success() {
            return Err(AppError::Upstream(format!(
                "recording list returned {}",
                response.status()
            )));
        }
        let spans: Vec<RecordingSpan> = response
            .json()
            .await
            .map_err(|_| AppError::Upstream("recording list response was invalid".into()))?;
        split_recording_spans(spans)
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

fn split_recording_spans(spans: Vec<RecordingSpan>) -> Result<Vec<RecordingSpan>> {
    let mut windows = Vec::new();
    for span in spans {
        let start = DateTime::parse_from_rfc3339(&span.start)
            .map_err(|_| AppError::Upstream("recording span start was invalid".into()))?;
        if !span.duration.is_finite() || span.duration <= 0.0 {
            return Err(AppError::Upstream(
                "recording span duration was invalid".into(),
            ));
        }
        let mut remaining = span.duration;
        let mut offset_seconds = 0_i64;
        while remaining > 0.0 {
            if windows.len() >= MAX_RECORDING_WINDOWS {
                return Err(AppError::Upstream("recording list was too large".into()));
            }
            let current = start
                .checked_add_signed(ChronoDuration::seconds(offset_seconds))
                .ok_or_else(|| AppError::Upstream("recording span time was invalid".into()))?;
            let duration = remaining.min(MAX_PLAYBACK_SECONDS);
            windows.push(RecordingSpan {
                start: current.to_rfc3339(),
                duration,
                url: span.url.clone(),
            });
            remaining -= duration;
            offset_seconds += MAX_PLAYBACK_SECONDS as i64;
        }
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::{split_recording_spans, MediaMtxClient, RecordingSpan, MAX_PLAYBACK_SECONDS};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn missing_recording_segments_are_an_empty_search_result() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let responder = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 4096];
            let length = connection.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..length]);
            assert!(request.starts_with("GET /list?"));
            connection
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let http = reqwest::Client::new();
        let media = MediaMtxClient::new(
            http.clone(),
            http,
            format!("http://{address}"),
            format!("http://{address}"),
        );
        assert!(media
            .recordings("cam_test_main", None, None, "token")
            .await
            .unwrap()
            .is_empty());
        responder.await.unwrap();
    }

    #[test]
    fn long_recording_spans_are_split_into_playable_windows() {
        let windows = split_recording_spans(vec![RecordingSpan {
            start: "2026-10-05T00:00:00Z".into(),
            duration: 25.0 * 3600.0,
            url: None,
        }])
        .unwrap();
        assert_eq!(windows.len(), 5);
        assert!(windows
            .iter()
            .all(|span| span.duration <= MAX_PLAYBACK_SECONDS));
        assert_eq!(windows[4].start, "2026-10-06T00:00:00+00:00");
        assert_eq!(windows[4].duration, 3600.0);
        assert_eq!(
            windows.iter().map(|span| span.duration).sum::<f64>(),
            25.0 * 3600.0
        );
        assert!(split_recording_spans(vec![RecordingSpan {
            start: "invalid".into(),
            duration: 1.0,
            url: None,
        }])
        .is_err());
    }
}
