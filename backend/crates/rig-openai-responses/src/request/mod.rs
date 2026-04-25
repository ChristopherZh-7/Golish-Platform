//! Request building for the OpenAI Responses API.
//!
//! Pure conversion logic that takes a rig `CompletionRequest` plus the
//! model configuration and produces a `CreateResponse` ready to send to
//! OpenAI. No HTTP, no streaming — purely synchronous data
//! transformation, which is why most of the unit tests for this crate
//! live alongside this module.
//!
//! ## Layout
//!
//! - [`builder`]: top-level [`build_request`] entry point — orchestrates
//!   chat-history conversion, tool-list mapping, reasoning + temperature
//!   config, and stateless multi-turn (`encrypted_content`,
//!   `store: false`) wiring.
//! - [`conversion`]: per-message conversion to Responses API
//!   [`InputItem`]s — `convert_user_content`,
//!   `convert_assistant_content_to_items`, `convert_tool_definition`.
//! - [`reasoning`]: [`apply_additional_params_reasoning`] — late-stage
//!   override of the reasoning config from `additional_params["reasoning"]`,
//!   used by the agentic loop to tweak effort/summary per call.
//!
//! [`InputItem`]: async_openai::types::responses::InputItem

mod builder;
mod conversion;
mod reasoning;

#[cfg(test)]
mod tests;

pub(crate) use builder::build_request;
