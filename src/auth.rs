use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
struct CachedToken {
    token: String,
    expires_at: Instant,
}

#[derive(Clone, Debug)]
pub struct TokenManager {
    cache: Arc<RwLock<Option<CachedToken>>>,
    ttl: Duration,
    project_id: Arc<RwLock<String>>,
}

impl TokenManager {
    pub fn new() -> Self {
        let initial_project = Self::read_project_id_from_db().unwrap_or_else(|| "charming-craft-95w3k".to_string());
        info!("Antigravity Project ID: {}", initial_project);

        Self {
            cache: Arc::new(RwLock::new(None)),
            ttl: Duration::from_secs(50 * 60),
            project_id: Arc::new(RwLock::new(initial_project)),
        }
    }

    pub async fn get_token(&self) -> Result<String, anyhow::Error> {
        // 1. Read lock
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if Instant::now() < cached.expires_at {
                    return Ok(cached.token.clone());
                }
            }
        }

        // 2. Write lock
        let mut guard = self.cache.write().await;
        if let Some(cached) = guard.as_ref() {
            if Instant::now() < cached.expires_at {
                return Ok(cached.token.clone());
            }
        }

        info!("Fetching fresh Google Antigravity token via omp...");
        let token = Self::fetch_token_from_omp().await?;

        // Also refresh project ID if found
        if let Some(p) = Self::read_project_id_from_db() {
            let mut p_guard = self.project_id.write().await;
            *p_guard = p;
        }

        let cached = CachedToken {
            token: token.clone(),
            expires_at: Instant::now() + self.ttl,
        };
        *guard = Some(cached);
        info!("Token successfully refreshed and cached (TTL: 50m)");

        Ok(token)
    }

    pub async fn get_project_id(&self) -> String {
        self.project_id.read().await.clone()
    }

    pub async fn invalidate(&self) {
        warn!("Invalidating cached token due to upstream auth error");
        let mut guard = self.cache.write().await;
        *guard = None;
    }

    fn read_project_id_from_db() -> Option<String> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string());
        let db_path = format!("{}/.omp/agent/agent.db", home);
        if !std::path::Path::new(&db_path).exists() {
            return None;
        }

        // Run sqlite3 query safely without heavy C dependencies
        let output = std::process::Command::new("sqlite3")
            .arg(&db_path)
            .arg("SELECT credential_json FROM auth_credentials WHERE credential_json LIKE '%projectId%' LIMIT 1;")
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let raw = String::from_utf8_lossy(&output.stdout);
        let val: serde_json::Value = serde_json::from_str(&raw).ok()?;
        val.get("projectId")?.as_str().map(|s| s.to_string())
    }

    async fn fetch_token_from_omp() -> Result<String, anyhow::Error> {
        let output = tokio::process::Command::new("/run/current-system/sw/bin/omp")
            .args(["token", "google-antigravity"])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Failed to fetch omp token: {}", stderr);
            anyhow::bail!("omp token command failed: {}", stderr.trim());
        }

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            anyhow::bail!("omp token returned empty output");
        }

        Ok(raw)
    }
}
