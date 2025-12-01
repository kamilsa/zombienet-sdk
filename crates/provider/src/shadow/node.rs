use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
    time::Duration,
};

use anyhow::anyhow;
use async_trait::async_trait;
use serde::Serialize;
use support::fs::FileSystem;
use tokio::time::sleep;
use tracing::debug;

use super::{client::ShadowAgentClient, namespace::ShadowNamespace};
use crate::{
    constants::{NODE_CONFIG_DIR, NODE_DATA_DIR, NODE_RELAY_DATA_DIR, NODE_SCRIPTS_DIR},
    types::{ExecutionResult, RunCommandOptions, RunScriptOptions, TransferedFile},
    ProviderError, ProviderNode,
};

pub(super) struct ShadowNodeOptions<'a, FS>
where
    FS: FileSystem + Send + Sync + Clone + 'static,
{
    pub(super) namespace: &'a Weak<ShadowNamespace<FS>>,
    pub(super) namespace_base_dir: &'a PathBuf,
    pub(super) name: &'a str,
    pub(super) program: &'a str,
    pub(super) args: &'a [String],
    pub(super) env: &'a [(String, String)],
    pub(super) startup_files: &'a [TransferedFile],
    pub(super) created_paths: &'a [PathBuf],
    pub(super) filesystem: &'a FS,
    pub(super) node_log_path: Option<&'a PathBuf>,
    pub(super) agent_host: Option<String>,
    pub(super) agent_port: u16,
}

#[derive(Serialize)]
pub(super) struct ShadowNode<FS>
where
    FS: FileSystem + Send + Sync + Clone,
{
    #[serde(skip)]
    _namespace: Weak<ShadowNamespace<FS>>,
    name: String,
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    base_dir: PathBuf,
    config_dir: PathBuf,
    data_dir: PathBuf,
    relay_data_dir: PathBuf,
    scripts_dir: PathBuf,
    log_path: PathBuf,
    #[serde(skip)]
    filesystem: FS,
    provider_tag: String,
    #[serde(skip)]
    client: ShadowAgentClient,
}

impl<FS> ShadowNode<FS>
where
    FS: FileSystem + Send + Sync + Clone + 'static,
{
    pub(super) async fn new(
        options: ShadowNodeOptions<'_, FS>,
    ) -> Result<Arc<Self>, ProviderError> {
        let filesystem = options.filesystem.clone();

        // In Shadow, the filesystem pathing might need to be relative to the shadow data dir.
        // Here we stick to the pattern used by other providers for structure.
        let base_dir =
            PathBuf::from_iter([options.namespace_base_dir, &PathBuf::from(options.name)]);
        // We do not create local dirs because we delegate to the agent inside Shadow
        // or we rely on shadow.yaml binds.

        let base_dir_raw = base_dir.to_string_lossy();
        let config_dir = PathBuf::from(format!("{base_dir_raw}{NODE_CONFIG_DIR}"));
        let data_dir = PathBuf::from(format!("{base_dir_raw}{NODE_DATA_DIR}"));
        let relay_data_dir = PathBuf::from(format!("{base_dir_raw}{NODE_RELAY_DATA_DIR}"));
        let scripts_dir = PathBuf::from(format!("{base_dir_raw}{NODE_SCRIPTS_DIR}"));
        let log_path = options
            .node_log_path
            .cloned()
            .unwrap_or_else(|| base_dir.join(format!("{}.log", options.name)));

        let agent_host = options
            .agent_host
            .clone()
            .unwrap_or_else(|| options.name.to_string());
        let client = ShadowAgentClient::new(&agent_host, options.agent_port);

        let node = Arc::new(ShadowNode {
            _namespace: options.namespace.clone(),
            name: options.name.to_string(),
            program: options.program.to_string(),
            args: options.args.to_vec(),
            env: options.env.to_vec(),
            base_dir,
            config_dir,
            data_dir,
            relay_data_dir,
            scripts_dir,
            log_path,
            filesystem: filesystem.clone(),
            provider_tag: super::provider::PROVIDER_NAME.to_string(),
            client,
        });

        // Initialize node via Agent
        // 1. Create directories
        for path in options.created_paths {
            tracing::debug!(target: "zombienet_provider::shadow", node = %options.name, remote_path = %path.display(), "creating remote path via agent");
            let _ = node
                .run_command(
                    RunCommandOptions::new("mkdir")
                        .args(vec!["-p", &path.to_string_lossy()]),
                )
                .await?;
        }

        // 2. Transfer files (via agent or assuming shared FS if properly mounted in shadow.yaml)
        // For strict Shadow simulation without shared FS, we'd POST the file content to the Agent.
        for file in options.startup_files {
            tracing::debug!(target: "zombienet_provider::shadow", node = %options.name, local = %file.local_path.display(), remote = %file.remote_path.display(), "uploading startup file");
            node
                .send_file(&file.local_path, &file.remote_path, &file.mode)
                .await?;
        }

        // 3. Start the process
        node.start_process().await?;

        Ok(node)
    }

    async fn start_process(&self) -> Result<(), ProviderError> {
        self.client
            .spawn(
                &self.program,
                &self.args,
                &self.env,
            )
            .await
            .map_err(|err| ProviderError::NodeSpawningFailed(self.name.clone(), err))
    }

}

#[async_trait]
impl<FS> ProviderNode for ShadowNode<FS>
where
    FS: FileSystem + Send + Sync + Clone + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn args(&self) -> Vec<&str> {
        self.args.iter().map(|arg| arg.as_str()).collect()
    }

    fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    fn config_dir(&self) -> &PathBuf {
        &self.config_dir
    }

    fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    fn relay_data_dir(&self) -> &PathBuf {
        &self.relay_data_dir
    }

    fn scripts_dir(&self) -> &PathBuf {
        &self.scripts_dir
    }

    fn log_path(&self) -> &PathBuf {
        &self.log_path
    }

    fn log_cmd(&self) -> String {
        // No interactive log tailing in this provider usually
        format!("tail -f {:?}", self.log_path)
    }

    fn path_in_node(&self, file: &Path) -> PathBuf {
        PathBuf::from(file)
    }

    async fn logs(&self) -> Result<String, ProviderError> {
        self.client.logs().await.map_err(|e| ProviderError::GetLogsFailed(self.name.clone(), e))
    }

    async fn dump_logs(&self, local_dest: PathBuf) -> Result<(), ProviderError> {
        let logs = self.logs().await?;
        self.filesystem.write(local_dest, logs).await?;
        Ok(())
    }

    async fn run_command(
        &self,
        options: RunCommandOptions,
    ) -> Result<ExecutionResult, ProviderError> {
        debug!("Running command on shadow node {}: {:?}", self.name, options);
        self.client.exec(&options.program, &options.args, &options.env)
            .await
            .map_err(|e| ProviderError::RunCommandError(options.program, self.name.clone(), e))
    }

    async fn run_script(
        &self,
        _options: RunScriptOptions,
    ) -> Result<ExecutionResult, ProviderError> {
        // Upload script then execute
        // Implementation omitted for brevity, similar to Docker provider logic
        Err(ProviderError::RunScriptError(self.name.clone(), anyhow!("Not implemented")))
    }

    async fn send_file(
        &self,
        local_file_path: &Path,
        remote_file_path: &Path,
        mode: &str,
    ) -> Result<(), ProviderError> {
        // Read local file
        let content = self
            .filesystem
            .read(local_file_path)
            .await
            .map_err(|err| {
                tracing::error!(target: "zombienet_provider::shadow", node = %self.name, local = %local_file_path.display(), remote = %remote_file_path.display(), "failed to read local file: {err}");
                err
            })?;
        self.client.upload_file(remote_file_path, &content, mode)
            .await
            .map_err(|e| ProviderError::SendFile(self.name.clone(), remote_file_path.to_string_lossy().to_string(), e))
    }

    async fn receive_file(
        &self,
        _remote_file_path: &Path,
        _local_file_path: &Path,
    ) -> Result<(), ProviderError> {
        // Optional: Implement download via agent
        Ok(())
    }

    async fn ip(&self) -> Result<IpAddr, ProviderError> {
        // In shadow, we can often rely on DNS.
        // If exact IP is needed, we could resolve `self.name`
        // For now, return a placeholder or resolve via standard library
        // since we are IN the simulation.
        let addrs = tokio::net::lookup_host(format!("{}:0", self.name))
            .await
            .map_err(|e| ProviderError::InvalidConfig(format!("Failed to resolve host {}: {}", self.name, e)))?;

        for addr in addrs {
            if let IpAddr::V4(ipv4) = addr.ip() {
                if ipv4.octets()[0] == 10 {
                    return Ok(IpAddr::V4(ipv4));
                }
            }
        }
        
        Err(ProviderError::InvalidConfig(format!("No 10.x.x.x IP found for {}", self.name)))
    }

    async fn pause(&self) -> Result<(), ProviderError> {
        self.client.pause().await.map_err(|e| ProviderError::PauseNodeFailed(self.name.clone(), e))
    }

    async fn resume(&self) -> Result<(), ProviderError> {
        self.client.resume().await.map_err(|e| ProviderError::ResumeNodeFailed(self.name.clone(), e))
    }

    async fn restart(&self, after: Option<Duration>) -> Result<(), ProviderError> {
        if let Some(d) = after {
            sleep(d).await;
        }
        self.client.restart().await.map_err(|e| ProviderError::RestartNodeFailed(self.name.clone(), e))
    }

    async fn destroy(&self) -> Result<(), ProviderError> {
        self.client.kill().await.map_err(|e| ProviderError::DestroyNodeFailed(self.name.clone(), e))
    }
}
