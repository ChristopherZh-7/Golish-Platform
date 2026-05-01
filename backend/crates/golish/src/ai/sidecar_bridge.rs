//! Bridge between the `golish-sidecar` crate and the `SessionCaptureBackend`
//! trait defined in `golish-ai`.

use std::sync::Arc;

use golish_ai::sidecar_trait::{AiEventProcessor, EndedSessionInfo, SessionCaptureBackend};
use golish_core::events::AiEvent;
use golish_sidecar::capture::CaptureContext;
use golish_sidecar::events::SessionEvent;
use golish_sidecar::SidecarState;

pub struct SidecarCaptureBackend {
    state: Arc<SidecarState>,
}

impl SidecarCaptureBackend {
    pub fn new(state: Arc<SidecarState>) -> Self {
        Self { state }
    }
}

#[async_trait::async_trait]
impl SessionCaptureBackend for SidecarCaptureBackend {
    fn current_session_id(&self) -> Option<String> {
        self.state.current_session_id()
    }

    fn start_session(&self, initial_request: &str) -> anyhow::Result<String> {
        self.state.start_session(initial_request)
    }

    fn end_session(&self) -> anyhow::Result<Option<EndedSessionInfo>> {
        self.state.end_session().map(|opt| {
            opt.map(|meta| EndedSessionInfo {
                session_id: meta.session_id,
            })
        })
    }

    fn capture_user_prompt(&self, session_id: &str, text: &str) {
        self.state
            .capture(SessionEvent::user_prompt(session_id.to_string(), text));
    }

    fn capture_ai_response(&self, session_id: &str, text: &str) {
        self.state
            .capture(SessionEvent::ai_response(session_id.to_string(), text));
    }

    fn capture_event(&self, event: &AiEvent) {
        let mut ctx = CaptureContext::new(self.state.clone());
        ctx.process(event);
    }

    fn create_event_processor(&self) -> Box<dyn AiEventProcessor> {
        Box::new(SidecarEventProcessor {
            ctx: CaptureContext::new(self.state.clone()),
        })
    }

    async fn get_injectable_context(&self) -> anyhow::Result<Option<String>> {
        self.state.get_injectable_context().await
    }
}

struct SidecarEventProcessor {
    ctx: CaptureContext,
}

impl AiEventProcessor for SidecarEventProcessor {
    fn process(&mut self, event: &AiEvent) {
        self.ctx.process(event);
    }
}
