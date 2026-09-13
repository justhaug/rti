//! YouTube publishing (Data API v3, resumable upload). Uploads are private
//! by default; publishing is a separate, explicit step so a human approves
//! what RTI posts. Configure with OAuth2 client id/secret + refresh token
//! in environment variables (see docs/media.md).

use std::path::Path;
use std::time::Duration;

pub use rti_core::config::YoutubeConfig;

pub struct YoutubeClient {
    agent: ureq::Agent,
    cfg: YoutubeConfig,
    base: String,
    token_url: String,
}

impl YoutubeClient {
    pub fn new(cfg: &YoutubeConfig) -> YoutubeClient {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(600)))
            .build()
            .into();
        YoutubeClient {
            agent,
            cfg: cfg.clone(),
            base: "https://www.googleapis.com".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
        }
    }

    /// Point at a mock server (tests).
    pub fn with_endpoints(mut self, base: &str, token_url: &str) -> Self {
        self.base = base.trim_end_matches('/').to_string();
        self.token_url = token_url.to_string();
        self
    }

    pub fn is_configured(&self) -> bool {
        [
            &self.cfg.client_id_env,
            &self.cfg.client_secret_env,
            &self.cfg.refresh_token_env,
        ]
        .iter()
        .all(|e| std::env::var(e).map(|v| !v.is_empty()).unwrap_or(false))
    }

    fn access_token(&self) -> anyhow::Result<String> {
        let id = std::env::var(&self.cfg.client_id_env)?;
        let secret = std::env::var(&self.cfg.client_secret_env)?;
        let refresh = std::env::var(&self.cfg.refresh_token_env)?;
        let form = [
            ("client_id", id.as_str()),
            ("client_secret", secret.as_str()),
            ("refresh_token", refresh.as_str()),
            ("grant_type", "refresh_token"),
        ];
        let v: serde_json::Value = self
            .agent
            .post(&self.token_url)
            .send_form(form)?
            .body_mut()
            .read_json()?;
        v["access_token"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("no access_token in OAuth response: {v}"))
    }

    /// Resumable upload of `path`; returns the YouTube video id.
    pub fn upload(
        &self,
        path: &Path,
        title: &str,
        description: &str,
        tags: &[String],
        privacy: &str,
    ) -> anyhow::Result<String> {
        let token = self.access_token()?;
        let meta = serde_json::json!({
            "snippet": {"title": title.chars().take(100).collect::<String>(), "description": description.chars().take(5000).collect::<String>(), "tags": tags, "categoryId": self.cfg.category_id},
            "status": {"privacyStatus": privacy, "selfDeclaredMadeForKids": false}
        });
        let bytes = std::fs::read(path)?;
        let init = self
            .agent
            .post(format!(
                "{}/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status",
                self.base
            ))
            .header("Authorization", &format!("Bearer {token}"))
            .header("X-Upload-Content-Length", &bytes.len().to_string())
            .header("X-Upload-Content-Type", "video/mp4")
            .send_json(&meta)?;
        let location = init
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("no resumable upload location"))?;
        let mut resp = self
            .agent
            .put(&location)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Content-Type", "video/mp4")
            .send(&bytes[..])?;
        let v: serde_json::Value = resp.body_mut().read_json()?;
        v["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("upload response without id: {v}"))
    }

    /// Change privacy (e.g. private → public) = publish.
    pub fn set_privacy(&self, video_id: &str, privacy: &str) -> anyhow::Result<()> {
        let token = self.access_token()?;
        let body = serde_json::json!({"id": video_id, "status": {"privacyStatus": privacy}});
        self.agent
            .put(format!("{}/youtube/v3/videos?part=status", self.base))
            .header("Authorization", &format!("Bearer {token}"))
            .send_json(&body)?;
        Ok(())
    }
}
