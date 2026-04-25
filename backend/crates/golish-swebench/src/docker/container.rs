//! Testbed container lifecycle: start/stop, applying patches, and `docker exec` helper.

use std::path::Path;

use anyhow::{Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, RemoveContainerOptions, StartContainerOptions,
};
use bollard::models::{HostConfig, Mount, MountTypeEnum};
use futures::StreamExt;
use tracing::{debug, info};

use crate::types::SWEBenchInstance;

use super::DockerExecutor;

impl DockerExecutor {
    /// Start a testbed container that stays running for agent interaction.
    ///
    /// This starts a container with the workspace mounted, allowing the agent
    /// to run commands (like pytest) inside the container via `docker exec`.
    ///
    /// # Arguments
    /// * `instance` - The SWE-bench instance
    /// * `workspace` - Path to the workspace containing the repository
    ///
    /// # Returns
    /// * Container name that can be used with `docker exec`
    /// * Returns error with "IMAGE_NOT_AVAILABLE" if no image is available
    pub async fn start_testbed_container(
        &self,
        instance: &SWEBenchInstance,
        workspace: &Path,
    ) -> Result<String> {
        let image = match self.find_or_pull_image(instance).await? {
            Some(img) => img,
            None => {
                anyhow::bail!(
                    "IMAGE_NOT_AVAILABLE: No Docker image available for instance {}",
                    instance.instance_id
                );
            }
        };

        let container_name = format!(
            "swebench-testbed-{}",
            instance.instance_id.replace("__", "-")
        );

        if self
            .client
            .inspect_container(&container_name, None)
            .await
            .is_ok()
        {
            info!("Removing existing container: {}", container_name);
            let remove_options = Some(RemoveContainerOptions {
                force: true,
                v: true,
                ..Default::default()
            });
            let _ = self
                .client
                .remove_container(&container_name, remove_options)
                .await;
        }

        let workspace_abs = workspace.canonicalize().with_context(|| {
            format!("Failed to resolve workspace path: {}", workspace.display())
        })?;

        let host_config = HostConfig {
            mounts: Some(vec![Mount {
                target: Some("/workspace".to_string()),
                source: Some(workspace_abs.to_string_lossy().to_string()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            }]),
            memory: Some(4 * 1024 * 1024 * 1024), // 4GB
            memory_swap: Some(4 * 1024 * 1024 * 1024),
            nano_cpus: Some(2_000_000_000), // 2 CPUs
            ..Default::default()
        };

        let config = Config {
            image: Some(image.clone()),
            cmd: Some(vec![
                "/bin/bash".to_string(),
                "-c".to_string(),
                "sleep infinity".to_string(),
            ]),
            working_dir: Some("/workspace/repo".to_string()),
            host_config: Some(host_config),
            env: Some(vec![
                "PYTHONDONTWRITEBYTECODE=1".to_string(),
                "PYTHONUNBUFFERED=1".to_string(),
            ]),
            ..Default::default()
        };

        let create_options = Some(CreateContainerOptions {
            name: &container_name,
            platform: None,
        });

        let container = self
            .client
            .create_container(create_options, config)
            .await
            .context("Failed to create testbed container")?;

        debug!("Created testbed container: {}", container.id);

        self.client
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await
            .context("Failed to start testbed container")?;

        info!(
            "Started testbed container: {} ({})",
            container_name,
            &container.id[..12]
        );

        Ok(container_name)
    }

    /// Stop and remove a testbed container.
    pub async fn stop_container(&self, container_name: &str) -> Result<()> {
        info!("Stopping testbed container: {}", container_name);

        let remove_options = Some(RemoveContainerOptions {
            force: true,
            v: true,
            ..Default::default()
        });

        self.client
            .remove_container(container_name, remove_options)
            .await
            .with_context(|| format!("Failed to remove container: {}", container_name))?;

        Ok(())
    }

    /// Apply a test patch to /testbed inside a running container.
    ///
    /// This is used to add the FAIL_TO_PASS tests to the container's testbed
    /// so the agent can run them during its work. Since we exclude test files
    /// from syncing (to prevent the agent from modifying them), we need to
    /// apply the test patch directly.
    pub async fn apply_test_patch_to_container(
        &self,
        container_name: &str,
        test_patch: &str,
    ) -> Result<()> {
        use bollard::exec::{CreateExecOptions, StartExecResults};

        if test_patch.is_empty() {
            return Ok(());
        }

        // Write patch to a temp file inside the container and apply it
        // We use base64 encoding to safely pass the patch content
        let patch_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            test_patch.as_bytes(),
        );

        let apply_cmd = format!(
            r#"
cd /testbed
echo '{}' | base64 -d > /tmp/test_patch.diff
if git apply --whitespace=nowarn --check /tmp/test_patch.diff 2>/dev/null; then
    git apply --whitespace=nowarn /tmp/test_patch.diff
    echo "Test patch applied successfully"
elif git apply --whitespace=nowarn --reverse --check /tmp/test_patch.diff 2>/dev/null; then
    echo "Test patch already applied"
else
    # Try with patch command as fallback
    patch -p1 --forward < /tmp/test_patch.diff 2>/dev/null || echo "Patch may already be applied"
fi
rm -f /tmp/test_patch.diff
"#,
            patch_b64
        );

        let exec_options = CreateExecOptions {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            cmd: Some(vec!["bash", "-c", &apply_cmd]),
            ..Default::default()
        };

        let exec = self
            .client
            .create_exec(container_name, exec_options)
            .await
            .context("Failed to create exec for test patch")?;

        match self.client.start_exec(&exec.id, None).await? {
            StartExecResults::Attached { mut output, .. } => {
                while let Some(Ok(msg)) = output.next().await {
                    match msg {
                        bollard::container::LogOutput::StdOut { message } => {
                            debug!(
                                "Test patch output: {}",
                                String::from_utf8_lossy(&message).trim()
                            );
                        }
                        bollard::container::LogOutput::StdErr { message } => {
                            debug!(
                                "Test patch stderr: {}",
                                String::from_utf8_lossy(&message).trim()
                            );
                        }
                        _ => {}
                    }
                }
            }
            StartExecResults::Detached => {}
        }

        Ok(())
    }

    /// Get the docker exec command prefix for running commands in a testbed container.
    ///
    /// Returns a command string that can be used with shell execution to run
    /// commands inside the container with the conda testbed environment activated.
    pub fn get_docker_exec_command(container_name: &str, command: &str) -> String {
        format!(
            "docker exec {} bash -c 'source /opt/miniconda3/etc/profile.d/conda.sh && conda activate testbed && {}'",
            container_name,
            command.replace('\'', "'\\''")
        )
    }
}
