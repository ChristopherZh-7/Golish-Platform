//! Image management: availability checks, pulling, and pulling with fallbacks.

use std::time::{Duration, Instant};

use anyhow::Result;
use bollard::image::CreateImageOptions;
use futures::StreamExt;
use tracing::{debug, info, warn};

use crate::types::SWEBenchInstance;

use super::DockerExecutor;

impl DockerExecutor {
    /// Check if Docker is available and running.
    pub async fn is_available(&self) -> bool {
        self.client.ping().await.is_ok()
    }

    /// Pull the Docker image for an instance.
    ///
    /// Returns Ok(true) if image was pulled/exists, Ok(false) if image not found.
    pub async fn pull_image(&self, instance: &SWEBenchInstance) -> Result<bool> {
        let image = instance.docker_image();
        info!("Pulling Docker image: {}", image);

        let options = Some(CreateImageOptions {
            from_image: image.clone(),
            ..Default::default()
        });

        let mut stream = self.client.create_image(options, None, None);
        let start = Instant::now();
        let mut had_error = false;

        while let Some(result) = stream.next().await {
            if start.elapsed() > Duration::from_secs(self.pull_timeout_secs) {
                anyhow::bail!(
                    "Image pull timed out after {} seconds",
                    self.pull_timeout_secs
                );
            }

            match result {
                Ok(info) => {
                    if let Some(status) = info.status {
                        debug!("Pull status: {}", status);
                    }
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("already exists") {
                        debug!("Image already exists");
                        return Ok(true);
                    }
                    if err_str.contains("404")
                        || err_str.contains("not found")
                        || err_str.contains("No such image")
                    {
                        warn!("Image not available: {}", image);
                        return Ok(false);
                    }
                    warn!("Pull warning: {}", e);
                    had_error = true;
                }
            }
        }

        if self.image_exists(instance).await {
            info!("Successfully pulled image: {}", image);
            Ok(true)
        } else if had_error {
            Ok(false)
        } else {
            Ok(true)
        }
    }

    /// Check if an image exists locally (tries all alternatives).
    pub async fn image_exists(&self, instance: &SWEBenchInstance) -> bool {
        for image in instance.docker_image_alternatives() {
            if self.client.inspect_image(&image).await.is_ok() {
                return true;
            }
        }
        false
    }

    /// Find which image is available for an instance (local or pullable).
    async fn find_available_image(&self, instance: &SWEBenchInstance) -> Option<String> {
        for image in instance.docker_image_alternatives() {
            if self.client.inspect_image(&image).await.is_ok() {
                return Some(image);
            }
        }
        None
    }

    /// Try to find or pull an image, checking all alternatives.
    pub(super) async fn find_or_pull_image(
        &self,
        instance: &SWEBenchInstance,
    ) -> Result<Option<String>> {
        if let Some(image) = self.find_available_image(instance).await {
            info!("Using cached image: {}", image);
            return Ok(Some(image));
        }

        for image in instance.docker_image_alternatives() {
            info!("Trying to pull image: {}", image);
            if self.try_pull_image(&image).await? {
                return Ok(Some(image));
            }
        }

        Ok(None)
    }

    /// Try to pull a specific image. Returns Ok(true) if successful, Ok(false) if not found.
    async fn try_pull_image(&self, image: &str) -> Result<bool> {
        let options = Some(CreateImageOptions {
            from_image: image.to_string(),
            ..Default::default()
        });

        let mut stream = self.client.create_image(options, None, None);
        let start = Instant::now();

        while let Some(result) = stream.next().await {
            if start.elapsed() > Duration::from_secs(self.pull_timeout_secs) {
                anyhow::bail!(
                    "Image pull timed out after {} seconds",
                    self.pull_timeout_secs
                );
            }

            match result {
                Ok(info) => {
                    if let Some(status) = info.status {
                        debug!("Pull status: {}", status);
                    }
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("already exists") {
                        return Ok(true);
                    }
                    if err_str.contains("404")
                        || err_str.contains("not found")
                        || err_str.contains("No such image")
                        || err_str.contains("manifest unknown")
                    {
                        debug!("Image not available: {}", image);
                        return Ok(false);
                    }
                    warn!("Pull warning for {}: {}", image, e);
                }
            }
        }

        Ok(self.client.inspect_image(image).await.is_ok())
    }
}
