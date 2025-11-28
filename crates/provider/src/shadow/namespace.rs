use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use async_trait::async_trait;
use support::fs::FileSystem;
use tokio::sync::RwLock;
use tracing::{trace, warn};
use uuid::Uuid;

use super::{
    node::{ShadowNode, ShadowNodeOptions},
    provider::ShadowProvider,
};
use crate::{
    constants::NAMESPACE_PREFIX,
    types::{GenerateFilesOptions, ProviderCapabilities, SpawnNodeOptions},
    DynNode, ProviderError, ProviderNamespace, ProviderNode,
};

pub(super) struct ShadowNamespace<FS>
where
    FS: FileSystem + Send + Sync + Clone,
{
    weak: Weak<ShadowNamespace<FS>>,
    name: String,
    provider: Weak<ShadowProvider<FS>>,
    base_dir: PathBuf,
    capabilities: ProviderCapabilities,
    filesystem: FS,
    pub(super) nodes: RwLock<HashMap<String, Arc<ShadowNode<FS>>>>,
    agent_port: u16,
}

impl<FS> ShadowNamespace<FS>
where
    FS: FileSystem + Send + Sync + Clone + 'static,
{
    pub(super) async fn new(
        provider: &Weak<ShadowProvider<FS>>,
        tmp_dir: &PathBuf,
        capabilities: &ProviderCapabilities,
        filesystem: &FS,
        custom_base_dir: Option<&Path>,
        agent_port: u16,
    ) -> Result<Arc<Self>, ProviderError> {
        let name = format!("{}{}", NAMESPACE_PREFIX, Uuid::new_v4());
        let base_dir = if let Some(custom_base_dir) = custom_base_dir {
            if !filesystem.exists(custom_base_dir).await {
                filesystem.create_dir_all(custom_base_dir).await?;
            } else {
                warn!(
                    "⚠️ Using and existing directory {} as base dir",
                    custom_base_dir.to_string_lossy()
                );
            }
            PathBuf::from(custom_base_dir)
        } else {
            let base_dir = PathBuf::from_iter([tmp_dir, &PathBuf::from(&name)]);
            filesystem.create_dir(&base_dir).await?;
            base_dir
        };

        Ok(Arc::new_cyclic(|weak| ShadowNamespace {
            weak: weak.clone(),
            provider: provider.clone(),
            name,
            base_dir,
            capabilities: capabilities.clone(),
            filesystem: filesystem.clone(),
            nodes: RwLock::new(HashMap::new()),
            agent_port,
        }))
    }

    pub(super) async fn attach_to_live(
        provider: &Weak<ShadowProvider<FS>>,
        capabilities: &ProviderCapabilities,
        filesystem: &FS,
        custom_base_dir: &Path,
        name: &str,
        agent_port: u16,
    ) -> Result<Arc<Self>, ProviderError> {
        let base_dir = custom_base_dir.to_path_buf();

        Ok(Arc::new_cyclic(|weak| ShadowNamespace {
            weak: weak.clone(),
            provider: provider.clone(),
            name: name.to_string(),
            base_dir,
            capabilities: capabilities.clone(),
            filesystem: filesystem.clone(),
            nodes: RwLock::new(HashMap::new()),
            agent_port,
        }))
    }
}

#[async_trait]
impl<FS> ProviderNamespace for ShadowNamespace<FS>
where
    FS: FileSystem + Send + Sync + Clone + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    fn provider_name(&self) -> &str {
        super::provider::PROVIDER_NAME
    }

    async fn nodes(&self) -> HashMap<String, DynNode> {
        self.nodes
            .read()
            .await
            .iter()
            .map(|(name, node)| (name.clone(), node.clone() as DynNode))
            .collect()
    }

    async fn get_node_available_args(
        &self,
        (_command, _image): (String, Option<String>),
    ) -> Result<String, ProviderError> {
        // In shadow, we usually rely on pre-configured binaries.
        // Implementing this requires spawning a temp node via Agent to run --help.
        // For simplicity, we assume empty or implement dummy return.
        Ok("".to_string())
    }

    async fn spawn_node(&self, options: &SpawnNodeOptions) -> Result<DynNode, ProviderError> {
        trace!("spawn node options {options:?}");

        // In Shadow mode, the "name" of the node usually corresponds to the hostname
        // defined in the shadow.yaml topology.
        let node = ShadowNode::new(ShadowNodeOptions {
            namespace: &self.weak,
            namespace_base_dir: &self.base_dir,
            name: &options.name,
            program: &options.program,
            args: &options.args,
            env: &options.env,
            startup_files: &options.injected_files,
            created_paths: &options.created_paths,
            filesystem: &self.filesystem,
            node_log_path: options.node_log_path.as_ref(),
            agent_port: self.agent_port,
        })
        .await?;

        self.nodes
            .write()
            .await
            .insert(options.name.clone(), node.clone());

        Ok(node)
    }

    async fn spawn_node_from_json(
        &self,
        _json_value: &serde_json::Value,
    ) -> Result<DynNode, ProviderError> {
        // Shadow re-attachment from JSON is not fully supported in this version.
        Err(ProviderError::InvalidConfig("Spawn from JSON not implemented for Shadow".into()))
    }

    async fn generate_files(&self, _options: GenerateFilesOptions) -> Result<(), ProviderError> {
        // File generation logic using a temporary node via Agent
        // Not implemented in this basic version.
        Ok(())
    }

    async fn static_setup(&self) -> Result<(), ProviderError> {
        Ok(())
    }

    async fn destroy(&self) -> Result<(), ProviderError> {
        let mut names = vec![];

        for node in self.nodes.read().await.values() {
            node.destroy().await?;
            names.push(node.name().to_string());
        }

        let mut nodes = self.nodes.write().await;
        for name in names {
            nodes.remove(&name);
        }

        if let Some(provider) = self.provider.upgrade() {
            provider.namespaces.write().await.remove(&self.name);
        }

        Ok(())
    }
}
