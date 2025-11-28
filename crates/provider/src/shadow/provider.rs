use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use async_trait::async_trait;
use support::fs::FileSystem;
use tokio::sync::RwLock;

use super::namespace::ShadowNamespace;
use crate::{
    shared::helpers::extract_namespace_info, types::ProviderCapabilities, DynNamespace, Provider,
    ProviderError, ProviderNamespace,
};

pub const PROVIDER_NAME: &str = "shadow";

pub struct ShadowProvider<FS>
where
    FS: FileSystem + Send + Sync + Clone,
{
    weak: Weak<ShadowProvider<FS>>,
    capabilities: ProviderCapabilities,
    tmp_dir: PathBuf,
    filesystem: FS,
    pub(super) namespaces: RwLock<HashMap<String, Arc<ShadowNamespace<FS>>>>,
    // Configuration for the Shadow Agent (e.g., port)
    pub(super) agent_port: u16,
}

impl<FS> ShadowProvider<FS>
where
    FS: FileSystem + Send + Sync + Clone + 'static,
{
    pub fn new(filesystem: FS) -> Arc<Self> {
        let agent_port = std::env::var("ZOMBIENET_SHADOW_AGENT_PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse::<u16>()
            .expect("Invalid ZOMBIENET_SHADOW_AGENT_PORT");

        Arc::new_cyclic(|weak| ShadowProvider {
            weak: weak.clone(),
            capabilities: ProviderCapabilities {
                has_resources: false,
                requires_image: false,
                prefix_with_full_path: true,
                use_default_ports_in_cmd: false,
            },
            tmp_dir: std::env::temp_dir().canonicalize().unwrap_or_else(|_| std::env::temp_dir()),
            filesystem,
            namespaces: RwLock::new(HashMap::new()),
            agent_port,
        })
    }

    pub fn tmp_dir(mut self, tmp_dir: impl Into<PathBuf>) -> Self {
        let path = tmp_dir.into();
        self.tmp_dir = path.canonicalize().unwrap_or(path);
        self
    }
}

#[async_trait]
impl<FS> Provider for ShadowProvider<FS>
where
    FS: FileSystem + Send + Sync + Clone + 'static,
{
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    async fn namespaces(&self) -> HashMap<String, DynNamespace> {
        self.namespaces
            .read()
            .await
            .iter()
            .map(|(name, namespace)| (name.clone(), namespace.clone() as DynNamespace))
            .collect()
    }

    async fn create_namespace(&self) -> Result<DynNamespace, ProviderError> {
        let namespace = ShadowNamespace::new(
            &self.weak,
            &self.tmp_dir,
            &self.capabilities,
            &self.filesystem,
            None,
            self.agent_port,
        )
        .await?;

        self.namespaces
            .write()
            .await
            .insert(namespace.name().to_string(), namespace.clone());

        Ok(namespace)
    }

    async fn create_namespace_with_base_dir(
        &self,
        base_dir: &Path,
    ) -> Result<DynNamespace, ProviderError> {
        let namespace = ShadowNamespace::new(
            &self.weak,
            &self.tmp_dir,
            &self.capabilities,
            &self.filesystem,
            Some(base_dir),
            self.agent_port,
        )
        .await?;

        self.namespaces
            .write()
            .await
            .insert(namespace.name().to_string(), namespace.clone());

        Ok(namespace)
    }

    async fn create_namespace_from_json(
        &self,
        json_value: &serde_json::Value,
    ) -> Result<DynNamespace, ProviderError> {
        // Shadow integration might not support full re-attachment from JSON yet
        // but we keep the structure for compatibility.
        let (base_dir, name) = extract_namespace_info(json_value)?;

        let namespace = ShadowNamespace::attach_to_live(
            &self.weak,
            &self.capabilities,
            &self.filesystem,
            &base_dir,
            &name,
            self.agent_port,
        )
        .await?;

        self.namespaces
            .write()
            .await
            .insert(namespace.name().to_string(), namespace.clone());

        Ok(namespace)
    }
}
