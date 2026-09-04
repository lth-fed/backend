use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub struct Stats {
    pub authenticated: AtomicUsize,
    pub release_queued: AtomicUsize,
    pub reservation_queued: AtomicUsize,
    pub reserved: AtomicUsize,
    pub purchased: AtomicUsize,
    pub sold_out: AtomicUsize,
    pub errors: AtomicUsize,
    pub finished: AtomicUsize,
}

impl Stats {
    pub fn summary(&self, total: usize) -> String {
        format!(
            "clients={total} auth={} release-queued={} reservation-queued={} reserved={} purchased={} sold-out={} errors={} finished={}",
            self.authenticated.load(Ordering::Relaxed),
            self.release_queued.load(Ordering::Relaxed),
            self.reservation_queued.load(Ordering::Relaxed),
            self.reserved.load(Ordering::Relaxed),
            self.purchased.load(Ordering::Relaxed),
            self.sold_out.load(Ordering::Relaxed),
            self.errors.load(Ordering::Relaxed),
            self.finished.load(Ordering::Relaxed),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct SampleStatus {
    inner: Arc<Mutex<SampleStatusInner>>,
}

#[derive(Debug, Default)]
struct SampleStatusInner {
    release: Option<String>,
    status: String,
    next_poll: Option<String>,
}

impl SampleStatus {
    pub fn set(
        &self,
        release: Option<String>,
        status: impl Into<String>,
        next_poll: Option<String>,
    ) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.release = release;
            inner.status = status.into();
            inner.next_poll = next_poll;
        }
    }

    pub fn summary(&self) -> String {
        let Ok(inner) = self.inner.lock() else {
            return "sample unavailable".to_owned();
        };
        let mut summary = String::new();
        let _ = write!(
            summary,
            "sample: status={} release={}",
            inner.status,
            inner.release.as_deref().unwrap_or("unknown")
        );
        if let Some(next_poll) = &inner.next_poll {
            let _ = write!(summary, " next-poll={next_poll}");
        }
        summary
    }
}
