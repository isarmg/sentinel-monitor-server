//! Validate stored camera metadata against the current application contract.
use anyhow::ensure;
use futures_util::TryStreamExt;
use sqlx::{Row, SqlitePool};

fn validate_camera_values(capabilities: &str, streams: &str) -> anyhow::Result<()> {
    ensure!(
        capabilities.len() <= 4096 && streams.len() <= 64 * 1024,
        "stored camera metadata exceeds its budget"
    );
    serde_json::from_str::<crate::models::DeviceCapabilities>(capabilities)?;
    let streams: Vec<crate::models::StreamDescriptor> = serde_json::from_str(streams)?;
    ensure!(
        streams.len() <= 16,
        "stored stream count exceeds its budget"
    );
    Ok(())
}

pub(crate) async fn validate_current_camera_data(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut rows = sqlx::query("SELECT capabilities_json,streams_json FROM cameras").fetch(pool);
    while let Some(row) = rows.try_next().await? {
        validate_camera_values(row.try_get(0)?, row.try_get(1)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_camera_rows_reject_corruption_and_excessive_metadata() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE cameras(capabilities_json TEXT NOT NULL,streams_json TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let capabilities =
            serde_json::to_string(&crate::models::DeviceCapabilities::default()).unwrap();
        sqlx::query("INSERT INTO cameras VALUES(?,?)")
            .bind(&capabilities)
            .bind("[]")
            .execute(&pool)
            .await
            .unwrap();
        validate_current_camera_data(&pool).await.unwrap();
        sqlx::query("INSERT INTO cameras VALUES(?,?)")
            .bind(r#"{"video":123}"#)
            .bind("[]")
            .execute(&pool)
            .await
            .unwrap();
        assert!(validate_current_camera_data(&pool).await.is_err());

        let stream = serde_json::json!({"profile":"main","video_codec":"h264","audio_codec":null,"width":1920,"height":1080,"frame_rate":25.0});
        let sixteen = serde_json::to_string(&vec![stream.clone(); 16]).unwrap();
        validate_camera_values(&capabilities, &sixteen).unwrap();
        let seventeen = serde_json::to_string(&vec![stream; 17]).unwrap();
        assert!(validate_camera_values(&capabilities, &seventeen).is_err());
        assert!(validate_camera_values(&capabilities, &" ".repeat(64 * 1024 + 1)).is_err());
        assert!(validate_camera_values(&(capabilities + &" ".repeat(4096)), "[]").is_err());
        pool.close().await;
    }
}
