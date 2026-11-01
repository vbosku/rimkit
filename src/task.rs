//! Background work: run a closure on its own thread, poll the result from the UI thread.
//! The UI never waits; it just shows a spinner/progress until `poll` returns the value.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

/// Wakes the UI (egui repaint) from any thread.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Handed to the job so it can report how far along it is.
#[derive(Clone)]
pub struct Progress {
    fraction: Arc<AtomicU32>,
    waker: Waker,
}

impl Progress {
    pub fn set(&self, done: u64, total: u64) {
        let f = if total == 0 { 0.0 } else { (done as f32 / total as f32).min(1.0) };
        self.fraction.store(f.to_bits(), Ordering::Relaxed);
        (self.waker)();
    }
}

pub struct Task<T> {
    pub label: String,
    rx: mpsc::Receiver<T>,
    progress: Progress,
}

impl<T: Send + 'static> Task<T> {
    pub fn spawn(label: impl Into<String>, waker: &Waker, job: impl FnOnce(&Progress) -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let progress = Progress { fraction: Arc::new(AtomicU32::new(0)), waker: waker.clone() };
        let p = progress.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job(&p));
            (p.waker)();
        });
        Self { label: label.into(), rx, progress }
    }

    /// The result, once (and only once) the job is done.
    pub fn poll(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// 0.0..=1.0 as last reported by the job.
    pub fn fraction(&self) -> f32 {
        f32::from_bits(self.progress.fraction.load(Ordering::Relaxed))
    }
}

/// Takes the finished result out of an optional task slot, clearing the slot.
pub fn take<T: Send + 'static>(slot: &mut Option<Task<T>>) -> Option<T> {
    let result = slot.as_ref()?.poll()?;
    *slot = None;
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn result_arrives_without_blocking_and_reports_progress() {
        let waker: Waker = Arc::new(|| {});
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let mut slot = Some(Task::spawn("job", &waker, move |p| {
            p.set(1, 2);
            go_rx.recv().unwrap();
            42
        }));
        let t = Instant::now();
        while slot.as_ref().unwrap().fraction() < 0.5 {
            assert!(t.elapsed() < Duration::from_secs(5));
        }
        assert_eq!(take(&mut slot), None, "not finished yet, must not block");
        go_tx.send(()).unwrap();
        let t = Instant::now();
        let value = loop {
            if let Some(v) = take(&mut slot) {
                break v;
            }
            assert!(t.elapsed() < Duration::from_secs(5));
        };
        assert_eq!(value, 42);
        assert!(slot.is_none());
    }
}
