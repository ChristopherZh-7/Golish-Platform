//! Model capability detection: vision support, thinking-history support,
//! temperature support, web search support.

mod capabilities;
mod helpers;
mod vision;

#[cfg(test)]
mod tests;

pub use capabilities::ModelCapabilities;
pub use helpers::{model_supports_temperature, openai_supports_web_search};
pub use vision::VisionCapabilities;
