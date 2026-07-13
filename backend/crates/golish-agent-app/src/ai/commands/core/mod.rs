//! AI commands grouped by domain.

pub mod chat;
pub mod operation_resume;
pub mod session;

pub use chat::*;
pub use session::*;

#[doc(hidden)]
pub use chat::{
    __cmd__clear_ai_conversation_session, __cmd__get_ai_conversation_length_session,
    __cmd__get_vision_capabilities, __cmd__send_ai_prompt_session,
    __cmd__send_ai_prompt_with_attachments, __cmd__signal_frontend_ready,
};
#[doc(hidden)]
pub use session::{
    __cmd__ai_cancel_background_job, __cmd__cancel_ai_generation, __cmd__get_session_ai_config,
    __cmd__init_ai_session, __cmd__is_ai_session_initialized, __cmd__shutdown_ai_session,
};
