use std::{num::NonZeroU64, sync::atomic::AtomicU64};

use std::sync::atomic::Ordering;

use crate::ExperimentRunHandle;

#[derive(Debug)]
pub struct ProgressIdAllocator {
    next: AtomicU64,
}

impl ProgressIdAllocator {
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    pub fn next(&self) -> ProgressId {
        let id = self.next.fetch_add(1, Ordering::Relaxed);

        // Starts at 1, so this should only fail after overflow/wraparound.
        let id = NonZeroU64::new(id).expect("progress id allocator overflowed or produced zero");

        ProgressId(id)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ProgressEvent {
    Started {
        node: ProgressNode,
    },
    Updated {
        id: ProgressId,
        current: u64,
        total: Option<u64>,
    },
    Finished {
        id: ProgressId,
        status: ProgressStatus,
        message: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ProgressStatus {
    Success,
    Abandonned,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
pub struct ProgressId(NonZeroU64);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProgressNode {
    pub id: ProgressId,
    pub parent: Option<ProgressId>,
    pub name: String,
    pub unit: Option<String>,
    pub total: Option<u64>,
    pub attributes: serde_json::Map<String, serde_json::Value>,
}

pub trait ProgressEventReporter {
    fn report(&self, event: ProgressEvent);
}

pub struct ProgressBuilder {
    reporter: Box<dyn ProgressEventReporter>,
    id_allocator: ProgressIdAllocator,
    parent: Option<ProgressId>,
    name: String,
    unit: Option<String>,
    total: Option<u64>,
    attributes: serde_json::Map<String, serde_json::Value>,
}

impl ProgressBuilder {
    pub fn total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }

    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    pub fn attr<T>(mut self, key: impl Into<String>, value: T) -> Result<Self, serde_json::Error>
    where
        T: serde::Serialize,
    {
        self.attributes
            .insert(key.into(), serde_json::to_value(value)?);
        Ok(self)
    }

    pub fn start(self) -> ProgressGuard {
        let id = self.id_allocator.next();

        let node = ProgressNode {
            id,
            parent: self.parent,
            name: self.name,
            total: self.total,
            unit: self.unit,
            attributes: self.attributes,
        };

        self.reporter
            .report(ProgressEvent::Started { node: node.clone() });

        ProgressGuard::new(self.reporter, node)
    }
}

pub struct ProgressGuard {
    reporter: Box<dyn ProgressEventReporter>,
    node: ProgressNode,
    current: u64,
}

impl ProgressGuard {
    pub fn new(reporter: Box<dyn ProgressEventReporter>, node: ProgressNode) -> Self {
        Self {
            reporter,
            node,
            current: 0,
        }
    }

    pub fn inc(&mut self, delta: u64) {
        self.current = self.current.saturating_add(delta);

        self.reporter.report(ProgressEvent::Updated {
            id: self.node.id.clone(),
            current: self.current,
            total: self.node.total,
        });
    }

    pub fn set(&mut self, current: u64) {
        self.current = current;

        self.reporter.report(ProgressEvent::Updated {
            id: self.node.id.clone(),
            current,
            total: self.node.total,
        });
    }

    pub fn finish(self) {
        self.finish_inner(ProgressStatus::Success, None);
    }

    pub fn finish_with_message(self, message: impl Into<String>) {
        self.finish_inner(ProgressStatus::Success, Some(message.into()));
    }

    pub fn abandon(self) {
        self.finish_inner(ProgressStatus::Abandonned, None);
    }

    pub fn abandon_with_message(self, message: impl Into<String>) {
        self.finish_inner(ProgressStatus::Abandonned, Some(message.into()));
    }

    fn finish_inner(&self, status: ProgressStatus, message: Option<String>) {
        self.reporter.report(ProgressEvent::Finished {
            id: self.node.id.clone(),
            status,
            message,
        });
    }
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        self.finish_inner(ProgressStatus::Abandonned, None);
    }
}
