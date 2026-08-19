//! Panic capture for experiment runs.
//!
//! A process-wide hook, chained onto whatever was installed before it, formats
//! every panic and keeps it in two places: a thread-local slot the panicking
//! thread's run reads back as its failure reason while it unwinds, and an
//! error log line on every run that opted into watching, so a panic on a
//! worker thread that never unwinds the run still leaves a trace on it.

use std::cell::RefCell;
use std::panic::PanicHookInfo;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once};

use crate::ExperimentRunHandle;

static INSTALL: Once = Once::new();
static WATCHERS: Mutex<Vec<(u64, ExperimentRunHandle)>> = Mutex::new(Vec::new());
static WATCHER_IDS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
    static IN_HOOK: RefCell<bool> = const { RefCell::new(false) };
}

/// Install the chained hook. Idempotent, and cheap enough to call from every
/// run construction so the failure-reason slot works without any opt-in.
pub(crate) fn install() {
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            record(info);
            previous(info);
        }));
    });
}

/// What the panicking thread recorded, taken so an unwinding run reports it once.
pub(crate) fn take_thread_panic() -> Option<String> {
    LAST_PANIC.with(|slot| slot.borrow_mut().take())
}

/// Stream every panic on any thread to `handle` as an error log line until the
/// returned guard drops.
pub(crate) fn watch(handle: ExperimentRunHandle) -> PanicWatch {
    install();
    let id = WATCHER_IDS.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut watchers) = WATCHERS.lock() {
        watchers.push((id, handle));
    }
    PanicWatch { id }
}

/// Keeps a run subscribed to panic reporting; dropping it unsubscribes.
pub struct PanicWatch {
    id: u64,
}

impl Drop for PanicWatch {
    fn drop(&mut self) {
        if let Ok(mut watchers) = WATCHERS.lock() {
            watchers.retain(|(id, _)| *id != self.id);
        }
    }
}

/// Resets the reentrancy flag even if a watcher's log emission panics.
struct HookFlagReset;

impl Drop for HookFlagReset {
    fn drop(&mut self) {
        IN_HOOK.with(|flag| *flag.borrow_mut() = false);
    }
}

fn record(info: &PanicHookInfo<'_>) {
    let reentered = IN_HOOK.with(|flag| flag.replace(true));
    if reentered {
        return;
    }
    let _reset = HookFlagReset;

    let line = describe(info);
    LAST_PANIC.with(|slot| *slot.borrow_mut() = Some(line.clone()));

    let watchers: Vec<ExperimentRunHandle> = WATCHERS
        .lock()
        .map(|entries| entries.iter().map(|(_, handle)| handle.clone()).collect())
        .unwrap_or_default();
    for handle in watchers {
        handle.log_error(line.clone());
    }
}

fn describe(info: &PanicHookInfo<'_>) -> String {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>").to_owned();
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_owned());

    match info.location() {
        Some(location) => format!("thread '{name}' panicked at {location}: {payload}"),
        None => format!("thread '{name}' panicked: {payload}"),
    }
}
