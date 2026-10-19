//! One cancellable preview timer per inspector, before any heavy work is queued.
use atlas_model::{app_state::BgResult, pane::TabId, preview_cache::PreviewRequest};

#[derive(Default)]
pub(crate) struct PreviewDelay {
    pending: Option<(TabId, PreviewRequest)>,
    generation: u64,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl PreviewDelay {
    pub(crate) fn schedule(
        &mut self,
        tab: TabId,
        request: PreviewRequest,
        rt: &tokio::runtime::Handle,
        tx: flume::Sender<BgResult>,
    ) {
        if self.pending.as_ref() == Some(&(tab, request.clone())) {
            return;
        }
        self.cancel();
        self.pending = Some((tab, request.clone()));
        let generation = self.generation;
        self.task = Some(rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let _ = tx.send(BgResult::PreviewDelayElapsed {
                tab,
                request,
                generation,
            });
        }));
    }

    pub(crate) fn accepts(&self, tab: TabId, request: &PreviewRequest, generation: u64) -> bool {
        self.generation == generation && self.pending.as_ref() == Some(&(tab, request.clone()))
    }

    pub(crate) fn cancel(&mut self) {
        self.pending = None;
        self.generation = self.generation.wrapping_add(1);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for PreviewDelay {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_model::layout_tree::TileId;
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn a_hundred_rapid_selections_queue_only_the_last_stable_preview() {
        let mut delay = PreviewDelay::default();
        let (tx, rx) = flume::unbounded();
        let rt = tokio::runtime::Handle::current();
        let tab = TileId(7);
        for index in 0..100 {
            delay.schedule(
                tab,
                PreviewRequest::Single(format!("photo-{index}.jpg").into()),
                &rt,
                tx.clone(),
            );
            tokio::task::yield_now().await;
            tokio::time::advance(Duration::from_millis(5)).await;
        }
        assert!(rx.is_empty());
        tokio::time::advance(Duration::from_millis(94)).await;
        tokio::task::yield_now().await;
        assert!(rx.is_empty(), "no preview before 100 ms of stability");
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        let BgResult::PreviewDelayElapsed {
            tab: ready_tab,
            request,
            generation,
        } = rx.try_recv().unwrap()
        else {
            panic!("unexpected background event")
        };
        assert_eq!(request, PreviewRequest::Single("photo-99.jpg".into()));
        assert!(delay.accepts(ready_tab, &request, generation));
        assert!(rx.is_empty());
        // An acknowledgement already queued before cancellation cannot load a
        // newly focused file, even if its path happens to be identical later.
        delay.cancel();
        assert!(!delay.accepts(ready_tab, &request, generation));
        delay.schedule(tab, request.clone(), &rt, tx);
        assert!(!delay.accepts(ready_tab, &request, generation));
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_renders_keep_the_timer_and_hiding_the_inspector_cancels_it() {
        let mut delay = PreviewDelay::default();
        let (tx, rx) = flume::unbounded();
        let rt = tokio::runtime::Handle::current();
        let request = PreviewRequest::Single("photo.jpg".into());
        delay.schedule(TileId(1), request.clone(), &rt, tx.clone());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(80)).await;
        delay.schedule(TileId(1), request.clone(), &rt, tx.clone());
        tokio::time::advance(Duration::from_millis(20)).await;
        tokio::task::yield_now().await;
        assert_eq!(rx.len(), 1);
        delay.schedule(TileId(2), request, &rt, tx);
        tokio::task::yield_now().await;
        delay.cancel();
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(rx.len(), 1);
    }
}
