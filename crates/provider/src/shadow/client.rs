use std::process::ExitStatus;
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize)]
struct SpawnRequest {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

#[derive(Serialize)]
struct UploadRequest {
    path: String,
    content_b64: String,
    mode: String,
}

#[derive(Deserialize)]
struct ExecResponse {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

pub struct ShadowAgentClient {
    base_url: String,
    http_client: Client,
}

impl ShadowAgentClient {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            base_url: format!("http://{host}:{port}"),
            http_client: Client::new(),
        }
    }

    pub async fn spawn(&self, program: &str, args: &[String], env: &[(String, String)]) -> Result<()> {
        let req = SpawnRequest {
            program: program.to_string(),
            args: args.to_vec(),
            env: env.to_vec(),
        };
        let res = self.http_client.post(format!("{}/spawn", self.base_url))
            .json(&req)
            .send()
            .await?;
        
        if !res.status().is_success() {
            return Err(anyhow!("Failed to spawn process: {}", res.status()));
        }
        Ok(())
    }

    pub async fn logs(&self) -> Result<String> {
        let res = self.http_client.get(format!("{}/logs", self.base_url))
            .send()
            .await?;
        Ok(res.text().await?)
    }

    pub async fn exec(&self, program: &str, args: &[String], env: &[(String, String)]) -> Result<Result<String, (ExitStatus, String)>> {
        let req = SpawnRequest {
            program: program.to_string(),
            args: args.to_vec(),
            env: env.to_vec(),
        };
        let res = self.http_client.post(format!("{}/exec", self.base_url))
            .json(&req)
            .send()
            .await?;

        if !res.status().is_success() {
             return Err(anyhow!("Exec request failed"));
        }

        let body: ExecResponse = res.json().await?;
        
        // Note: Creating ExitStatus from integer is platform specific/restricted in std.
        // We simulate the result structure here.
        if body.exit_code == 0 {
            Ok(Ok(body.stdout))
        } else {
            // We can't construct a real ExitStatus easily here, using a placeholder string for now
            // In a real implementation we might change ExecutionResult type or use a wrapper
            Err(anyhow!("Command failed with exit code {}: {}", body.exit_code, body.stderr))
        }
    }

    pub async fn upload_file(&self, path: &Path, content: &[u8], mode: &str) -> Result<()> {
        use base64::{Engine as _, engine::general_purpose};
        let req = UploadRequest {
            path: path.to_string_lossy().to_string(),
            content_b64: general_purpose::STANDARD.encode(content),
            mode: mode.to_string(),
        };
        let res = self.http_client.post(format!("{}/upload", self.base_url))
            .json(&req)
            .send()
            .await?;
        
        if !res.status().is_success() {
            return Err(anyhow!("Failed to upload file"));
        }
        Ok(())
    }

    pub async fn pause(&self) -> Result<()> {
        self.http_client.post(format!("{}/pause", self.base_url)).send().await?;
        Ok(())
    }

    pub async fn resume(&self) -> Result<()> {
        self.http_client.post(format!("{}/resume", self.base_url)).send().await?;
        Ok(())
    }

    pub async fn restart(&self) -> Result<()> {
        self.http_client.post(format!("{}/restart", self.base_url)).send().await?;
        Ok(())
    }

    pub async fn kill(&self) -> Result<()> {
        self.http_client.post(format!("{}/kill", self.base_url)).send().await?;
        Ok(())
    }
}
