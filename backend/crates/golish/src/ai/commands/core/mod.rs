//! AI commands grouped by domain.

mod chat;
mod lifecycle;
mod session;
mod tools;

pub use chat::*;
pub use lifecycle::*;
pub use session::*;
pub use tools::*;

#[doc(hidden)]
pub use chat::{
    __cmd__clear_ai_conversation_session, __cmd__get_ai_conversation_length_session,
    __cmd__get_vision_capabilities, __cmd__send_ai_prompt_session,
    __cmd__send_ai_prompt_with_attachments, __cmd__signal_frontend_ready,
};
#[doc(hidden)]
pub use lifecycle::{__cmd__init_ai_agent, __cmd__init_ai_agent_unified};
#[doc(hidden)]
pub use session::{
    __cmd__cancel_ai_generation, __cmd__get_session_ai_config, __cmd__init_ai_session,
    __cmd__is_ai_session_initialized, __cmd__shutdown_ai_session,
};
#[doc(hidden)]
pub use tools::{
    __cmd__execute_ai_tool, __cmd__get_available_tools, __cmd__is_ai_initialized,
    __cmd__list_sub_agents, __cmd__send_ai_prompt, __cmd__shutdown_ai_agent,
};
