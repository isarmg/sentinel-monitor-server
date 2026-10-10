pub struct Lifecycle {
    pub scope: xcss::server_runtime::WorkScope,
    pub pool: sqlx::SqlitePool,
    pub lock: std::sync::Mutex<Option<std::sync::Arc<crate::runtime_lock::ApplicationLock>>>,
}
#[async_trait::async_trait]
impl xcss::server_runtime::LifecycleParticipant for Lifecycle {
    fn quiesce(&self) {
        tracing::info!(event = "common.runtime.shutdown_started");
        self.scope.quiesce();
    }
    fn cancel_ordinary_work(&self) {
        self.scope.cancel_ordinary_work();
    }
    fn active_tasks(&self) -> (usize, usize) {
        (self.scope.work_tasks.len(), self.scope.commit_tasks.len())
    }
    async fn drain_requests(&self) {
        self.scope.drain_requests().await;
    }
    async fn drain_commits(&self) {
        self.scope.drain_commits().await;
    }
    async fn close_state(&self) -> Result<(), String> {
        self.pool.close().await;
        self.lock
            .lock()
            .map_err(|_| "runtime lock ownership is poisoned".to_string())?
            .take();
        Ok(())
    }
}
