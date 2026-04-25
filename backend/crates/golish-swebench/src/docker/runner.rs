//! Full `run_tests` orchestration: image -> container -> wait -> logs -> parse.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, LogOutput, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::models::{HostConfig, Mount, MountTypeEnum};
use futures::StreamExt;
use tracing::{debug, warn};

use crate::types::{SWEBenchInstance, TestExecutionResult};

use super::DockerExecutor;

impl DockerExecutor {
    /// Run tests for a SWE-bench instance.
    ///
    /// # Arguments
    /// * `instance` - The SWE-bench instance to test
    /// * `workspace` - Path to the workspace containing the modified repository
    ///
    /// Returns an error with "IMAGE_NOT_AVAILABLE" in the message if the Docker image
    /// doesn't exist for this instance.
    pub async fn run_tests(
        &self,
        instance: &SWEBenchInstance,
        workspace: &Path,
    ) -> Result<TestExecutionResult> {
        let start = Instant::now();

        let image = match self.find_or_pull_image(instance).await? {
            Some(img) => img,
            None => {
                anyhow::bail!(
                    "IMAGE_NOT_AVAILABLE: No Docker image available for instance {}. \
                     Tried: {:?}",
                    instance.instance_id,
                    instance.docker_image_alternatives()
                );
            }
        };
        let container_name = format!("swebench-{}-{}", instance.instance_id, uuid::Uuid::new_v4());

        // Write the test patch to the workspace so it can be applied inside Docker
        // The test patch adds new test cases that verify the fix
        let test_patch_path = workspace.join("repo").join(".swebench_test_patch.diff");
        if !instance.test_patch.is_empty() {
            std::fs::write(&test_patch_path, &instance.test_patch).with_context(|| {
                format!(
                    "Failed to write test patch to {}",
                    test_patch_path.display()
                )
            })?;
            debug!(
                "Wrote test patch ({} bytes) to {}",
                instance.test_patch.len(),
                test_patch_path.display()
            );
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

        let test_cmd = self.build_test_command(instance);

        let config = Config {
            image: Some(image.clone()),
            cmd: Some(vec!["/bin/bash".to_string(), "-c".to_string(), test_cmd]),
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
            .context("Failed to create container")?;

        debug!("Created container: {}", container.id);

        self.client
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await
            .context("Failed to start container")?;

        debug!("Started container: {}", container.id);

        let wait_result = tokio::time::timeout(
            Duration::from_secs(self.test_timeout_secs),
            self.wait_for_container(&container.id),
        )
        .await;

        let exit_code = match wait_result {
            Ok(Ok(code)) => code,
            Ok(Err(e)) => {
                warn!("Container wait error: {}", e);
                -1
            }
            Err(_) => {
                warn!("Container execution timed out");
                let _ = self
                    .client
                    .kill_container::<String>(&container.id, None)
                    .await;
                -1
            }
        };

        let (stdout, stderr) = self.get_container_logs(&container.id).await?;

        let remove_options = Some(RemoveContainerOptions {
            force: true,
            v: true,
            ..Default::default()
        });

        let _ = self
            .client
            .remove_container(&container.id, remove_options)
            .await;

        let (fail_to_pass_results, pass_to_pass_results) =
            Self::parse_test_results(instance, &stdout, &stderr);

        Ok(TestExecutionResult {
            execution_success: exit_code == 0,
            exit_code,
            stdout,
            stderr,
            fail_to_pass_results,
            pass_to_pass_results,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Wait for a container to finish.
    async fn wait_for_container(&self, container_id: &str) -> Result<i32> {
        let options = WaitContainerOptions {
            condition: "not-running",
        };

        let mut stream = self.client.wait_container(container_id, Some(options));

        if let Some(result) = stream.next().await {
            match result {
                Ok(response) => Ok(response.status_code as i32),
                Err(e) => Err(e.into()),
            }
        } else {
            anyhow::bail!("Container wait stream ended unexpectedly")
        }
    }

    /// Get container logs.
    async fn get_container_logs(&self, container_id: &str) -> Result<(String, String)> {
        let options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: false,
            ..Default::default()
        };

        let mut stdout = String::new();
        let mut stderr = String::new();

        let mut stream = self.client.logs(container_id, Some(options));

        while let Some(result) = stream.next().await {
            match result {
                Ok(log) => match log {
                    LogOutput::StdOut { message } => {
                        stdout.push_str(&String::from_utf8_lossy(&message));
                    }
                    LogOutput::StdErr { message } => {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                },
                Err(e) => {
                    warn!("Error reading logs: {}", e);
                }
            }
        }

        Ok((stdout, stderr))
    }

    /// Build the test command for an instance.
    ///
    /// Uses the repository-specific test runner from the official SWE-bench specs.
    /// Test names from FAIL_TO_PASS and PASS_TO_PASS are passed as-is without
    /// conversion - they're already in the correct format for each repository's
    /// test runner.
    fn build_test_command(&self, instance: &SWEBenchInstance) -> String {
        let fail_to_pass = instance.fail_to_pass_tests();
        let pass_to_pass = instance.pass_to_pass_tests();
        let test_cmd = instance.test_command();

        let all_tests: Vec<String> = fail_to_pass
            .iter()
            .chain(pass_to_pass.iter())
            .map(|t| format!("'{}'", t.replace('\'', "'\\''")))
            .collect();
        let test_args = all_tests.join(" ");

        let has_test_patch = !instance.test_patch.is_empty();

        let runner_name = match instance.test_runner() {
            crate::types::TestRunner::Django => "Django",
            crate::types::TestRunner::SymPy => "SymPy",
            crate::types::TestRunner::Sphinx => "Sphinx/tox",
            crate::types::TestRunner::Pytest => "pytest",
        };

        // Epoch AI containers have the repo at /testbed with the environment pre-configured.
        // We need to:
        // 1. Copy changes from /workspace/repo to /testbed
        // 2. Apply the test patch to /testbed
        // 3. Run tests from /testbed (to avoid path conflicts with conftest.py)
        format!(
            r#"
set -e

# Source conda and activate the testbed environment
source /opt/miniconda3/etc/profile.d/conda.sh
conda activate testbed

# Show which python we're using for debugging
which python
python --version

# Copy agent's changes from /workspace/repo to /testbed
# This preserves the container's environment while applying the agent's fixes
# IMPORTANT: Test files are EXCLUDED - they should not be modified by the agent
echo "=== Syncing changes from /workspace/repo to /testbed ==="
cd /workspace/repo

# Function to check if a file is a test file
is_test_file() {{
    local file="$1"
    case "$file" in
        tests/*|test/*|*/tests/*|*/test/*|test_*.py|*_test.py)
            return 0  # true - is a test file
            ;;
        *)
            return 1  # false - not a test file
            ;;
    esac
}}

# Find modified files and copy them to /testbed
# Use git diff to find changed files (if git is available)
if [ -d .git ]; then
    # Get list of modified/added files
    for file in $(git diff --name-only HEAD 2>/dev/null || git status --porcelain | awk '{{print $2}}'); do
        if [ -f "$file" ]; then
            # Skip test files - they should not be modified by the agent
            if is_test_file "$file"; then
                echo "  Skipped (test file): $file"
                continue
            fi
            # Create directory structure in /testbed if needed
            mkdir -p "/testbed/$(dirname "$file")"
            cp "$file" "/testbed/$file"
            echo "  Copied: $file"
        fi
    done
else
    # Fallback: copy all Python files that differ (excluding test files)
    echo "  No git repo, copying all modified Python files..."
    find . -name "*.py" -newer /testbed ! -path "*/tests/*" ! -path "*/test/*" ! -name "test_*.py" -exec cp --parents {{}} /testbed/ \; 2>/dev/null || true
fi

# Now work from /testbed
cd /testbed

# Apply the test patch if it exists
{apply_test_patch}

# Run tests using {runner_name}
echo "=== Running tests with {runner_name} ==="
{test_cmd} {test_args} 2>&1
"#,
            apply_test_patch = if has_test_patch {
                r#"
if [ -f /workspace/repo/.swebench_test_patch.diff ]; then
    echo "Applying test patch..."
    echo "=== Test patch contents (first 50 lines) ==="
    head -50 /workspace/repo/.swebench_test_patch.diff
    echo "=== End of test patch preview ==="

    # Check if patch is already applied (via git apply --reverse --check)
    if git apply --whitespace=nowarn --reverse --check /workspace/repo/.swebench_test_patch.diff 2>/dev/null; then
        echo "Test patch already applied (skipping)"
    # Try git apply first (strict)
    elif git apply --whitespace=nowarn --check /workspace/repo/.swebench_test_patch.diff 2>/dev/null; then
        git apply --whitespace=nowarn /workspace/repo/.swebench_test_patch.diff && echo "Test patch applied successfully (git apply)"
    # Try git apply with 3-way merge (handles some conflicts)
    elif git apply --whitespace=nowarn --3way /workspace/repo/.swebench_test_patch.diff 2>/dev/null; then
        echo "Test patch applied with 3-way merge"
    # Fallback to patch command (more lenient)
    elif patch -p1 --forward --ignore-whitespace < /workspace/repo/.swebench_test_patch.diff 2>/dev/null; then
        echo "Test patch applied successfully (patch -p1)"
    # Try patch with fuzz factor
    elif patch -p1 --forward --ignore-whitespace --fuzz=3 < /workspace/repo/.swebench_test_patch.diff 2>/dev/null; then
        echo "Test patch applied with fuzz (patch -p1 --fuzz=3)"
    else
        # Check if it might already be applied
        if git apply --whitespace=nowarn --reverse --check /workspace/repo/.swebench_test_patch.diff 2>/dev/null; then
            echo "Test patch already applied"
        else
            echo "WARNING: Could not apply test patch (may already be partially applied)"
        fi
    fi
else
    echo "No test patch file found"
fi
"#
            } else {
                "echo 'No test patch for this instance'"
            },
            runner_name = runner_name,
            test_cmd = test_cmd,
            test_args = test_args,
        )
    }
}
