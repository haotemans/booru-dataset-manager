use crate::app::BdmApp;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct AgentSync {
    last_poll: Option<Instant>,
    revision: i64,
    queued: BTreeSet<usize>,
}

impl BdmApp {
    pub(crate) fn poll_agent_changes(&mut self, ctx: &eframe::egui::Context) {
        if self.demo || self.is_capture() {
            return;
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.ingest_active()
            || self.workflow_active()
            || self.image_editor.active()
            || self.image_editor.visible
            || self
                .agent_sync
                .last_poll
                .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.agent_sync.last_poll = Some(Instant::now());
        let Some(path) = self.catalog.path.as_deref() else {
            return;
        };
        let Ok(state) = bdm_infrastructure::agent::snapshot(path) else {
            return;
        };
        if state.revision == self.agent_sync.revision {
            return;
        }
        if let Err(error) = self.reload_media() {
            self.form_error = error;
            return;
        }
        for id in &self.agent_sync.queued {
            self.pending_deletion.remove(id);
        }
        self.pending_deletion.extend(state.rejected.iter());
        self.agent_sync.queued = state.rejected;
        self.agent_sync.revision = state.revision;
        ctx.request_repaint();
    }
}
