use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::clock::Clock;
use crate::ports::{BlobStore, OutgoingMessage, PortError, Queue, ReceivedMessage, Scheduler};

#[derive(Debug)]
struct QueueEntry {
    id: u64,
    body: String,
    group: String,
    visible_at: DateTime<Utc>,
    receipt: Option<String>,
    receive_count: u32,
}

#[derive(Debug, Default)]
struct QueueState {
    next: u64,
    entries: Vec<QueueEntry>,
    seen: HashSet<String>,
}

pub struct MemoryQueue {
    clock: Arc<dyn Clock>,
    state: Mutex<QueueState>,
}

impl MemoryQueue {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Mutex::new(QueueState::default()),
        }
    }

    pub fn len(&self) -> usize {
        self.state.lock().expect("queue lock").entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bodies(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("queue lock")
            .entries
            .iter()
            .map(|e| e.body.clone())
            .collect()
    }
}

#[async_trait]
impl Queue for MemoryQueue {
    async fn send(&self, message: &OutgoingMessage) -> Result<(), PortError> {
        let now = self.clock.now();
        let mut state = self.state.lock().expect("queue lock");
        if !state.seen.insert(message.dedupe_id.clone()) {
            return Ok(());
        }
        state.next += 1;
        let id = state.next;
        let delay = message
            .delay
            .map(|d| chrono::Duration::from_std(d).expect("delays fit in chrono"))
            .unwrap_or_default();
        state.entries.push(QueueEntry {
            id,
            body: message.body.clone(),
            group: message.group.clone(),
            visible_at: now + delay,
            receipt: None,
            receive_count: 0,
        });
        Ok(())
    }

    async fn receive(&self, max: usize) -> Result<Vec<ReceivedMessage>, PortError> {
        let now = self.clock.now();
        let mut state = self.state.lock().expect("queue lock");
        let mut blocked = HashSet::new();
        let mut received = Vec::new();
        for entry in state.entries.iter_mut() {
            if received.len() >= max {
                break;
            }
            if blocked.contains(&entry.group) {
                continue;
            }
            blocked.insert(entry.group.clone());
            if entry.receipt.is_some() || entry.visible_at > now {
                continue;
            }
            entry.receive_count += 1;
            let receipt = format!("{}:{}", entry.id, entry.receive_count);
            entry.receipt = Some(receipt.clone());
            received.push(ReceivedMessage {
                body: entry.body.clone(),
                receipt,
                receive_count: entry.receive_count,
            });
        }
        Ok(received)
    }

    async fn delete(&self, receipt: &str) -> Result<(), PortError> {
        let mut state = self.state.lock().expect("queue lock");
        let before = state.entries.len();
        state
            .entries
            .retain(|e| e.receipt.as_deref() != Some(receipt));
        if state.entries.len() == before {
            return Err(PortError::new(
                "queue",
                format!("unknown receipt {receipt}"),
            ));
        }
        Ok(())
    }

    async fn release(&self, receipt: &str) -> Result<(), PortError> {
        let mut state = self.state.lock().expect("queue lock");
        let entry = state
            .entries
            .iter_mut()
            .find(|e| e.receipt.as_deref() == Some(receipt))
            .ok_or_else(|| PortError::new("queue", format!("unknown receipt {receipt}")))?;
        entry.receipt = None;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct ScheduleState {
    next: u64,
    by_time: BTreeMap<(DateTime<Utc>, u64), (String, OutgoingMessage)>,
    by_name: HashMap<String, (DateTime<Utc>, u64)>,
}

#[derive(Debug, Default)]
pub struct MemoryScheduler {
    state: Mutex<ScheduleState>,
}

impl MemoryScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take_due(&self, now: DateTime<Utc>) -> Vec<OutgoingMessage> {
        let mut state = self.state.lock().expect("scheduler lock");
        let due = state
            .by_time
            .range(..=(now, u64::MAX))
            .map(|(key, _)| *key)
            .collect::<Vec<_>>();
        due.into_iter()
            .map(|key| {
                let (name, message) = state.by_time.remove(&key).expect("due entry exists");
                state.by_name.remove(&name);
                message
            })
            .collect()
    }

    pub fn pending(&self) -> Vec<(String, DateTime<Utc>)> {
        self.state
            .lock()
            .expect("scheduler lock")
            .by_time
            .iter()
            .map(|((at, _), (name, _))| (name.clone(), *at))
            .collect()
    }
}

#[async_trait]
impl Scheduler for MemoryScheduler {
    async fn schedule(
        &self,
        name: &str,
        at: DateTime<Utc>,
        message: &OutgoingMessage,
    ) -> Result<(), PortError> {
        let mut state = self.state.lock().expect("scheduler lock");
        if state.by_name.contains_key(name) {
            return Ok(());
        }
        state.next += 1;
        let key = (at, state.next);
        state
            .by_time
            .insert(key, (name.to_string(), message.clone()));
        state.by_name.insert(name.to_string(), key);
        Ok(())
    }

    async fn cancel(&self, name: &str) -> Result<(), PortError> {
        let mut state = self.state.lock().expect("scheduler lock");
        if let Some(key) = state.by_name.remove(name) {
            state.by_time.remove(&key);
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct MemoryBlobStore {
    blobs: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn keys(&self) -> Vec<String> {
        let mut keys = self
            .blobs
            .lock()
            .expect("blob lock")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }
}

#[async_trait]
impl BlobStore for MemoryBlobStore {
    async fn put(&self, key: &str, body: Vec<u8>) -> Result<(), PortError> {
        self.blobs
            .lock()
            .expect("blob lock")
            .insert(key.to_string(), body);
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, PortError> {
        Ok(self.blobs.lock().expect("blob lock").get(key).cloned())
    }

    async fn delete(&self, key: &str) -> Result<(), PortError> {
        self.blobs.lock().expect("blob lock").remove(key);
        Ok(())
    }

    async fn delete_prefix(&self, prefix: &str, limit: usize) -> Result<usize, PortError> {
        let mut blobs = self.blobs.lock().expect("blob lock");
        let keys = blobs
            .keys()
            .filter(|key| key.starts_with(prefix))
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        for key in &keys {
            blobs.remove(key);
        }
        Ok(keys.len())
    }
}

#[derive(Debug, Clone)]
pub struct DirBlobStore {
    root: PathBuf,
}

impl DirBlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, key: &str) -> Result<PathBuf, PortError> {
        if key
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(PortError::new("blob store", format!("invalid key {key:?}")));
        }
        Ok(self.root.join(key))
    }
}

#[async_trait]
impl BlobStore for DirBlobStore {
    async fn put(&self, key: &str, body: Vec<u8>) -> Result<(), PortError> {
        let path = self.path(key)?;
        let parent = path.parent().expect("blob paths have a parent");
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| PortError::new("blob store", e.to_string()))?;
        tokio::fs::write(&path, body)
            .await
            .map_err(|e| PortError::new("blob store", e.to_string()))
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, PortError> {
        match tokio::fs::read(self.path(key)?).await {
            Ok(body) => Ok(Some(body)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(PortError::new("blob store", error.to_string())),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), PortError> {
        match tokio::fs::remove_file(self.path(key)?).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(PortError::new("blob store", error.to_string())),
        }
    }

    async fn delete_prefix(&self, prefix: &str, _limit: usize) -> Result<usize, PortError> {
        let root = self.path(prefix.trim_end_matches('/'))?;
        let failed = |error: std::io::Error| PortError::new("blob store", error.to_string());
        let mut files = 0;
        let mut pending = vec![root.clone()];
        while let Some(path) = pending.pop() {
            let mut entries = match tokio::fs::read_dir(&path).await {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(failed(error)),
            };
            while let Some(entry) = entries.next_entry().await.map_err(failed)? {
                if entry.file_type().await.map_err(failed)?.is_dir() {
                    pending.push(entry.path());
                } else {
                    files += 1;
                }
            }
        }
        if files > 0 {
            tokio::fs::remove_dir_all(&root).await.map_err(failed)?;
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use chrono::TimeZone;

    fn message(body: &str, group: &str) -> OutgoingMessage {
        OutgoingMessage {
            body: body.into(),
            group: group.into(),
            dedupe_id: body.into(),
            delay: None,
        }
    }

    #[tokio::test]
    async fn the_memory_queue_keeps_one_message_in_flight_per_group() {
        let clock = Arc::new(FakeClock::new(
            Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap(),
        ));
        let queue = MemoryQueue::new(clock);
        for (body, group) in [("a1", "a"), ("a2", "a"), ("b1", "b"), ("a1", "a")] {
            queue.send(&message(body, group)).await.unwrap();
        }
        assert_eq!(queue.len(), 3);
        let first = queue.receive(10).await.unwrap();
        assert_eq!(
            first.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
            ["a1", "b1"]
        );
        assert!(queue.receive(10).await.unwrap().is_empty());
        queue.release(&first[0].receipt).await.unwrap();
        let again = queue.receive(10).await.unwrap();
        assert_eq!((again[0].body.as_str(), again[0].receive_count), ("a1", 2));
        queue.delete(&again[0].receipt).await.unwrap();
        queue.delete(&first[1].receipt).await.unwrap();
        let next = queue.receive(10).await.unwrap();
        assert_eq!(next[0].body, "a2");
    }

    #[tokio::test]
    async fn the_memory_scheduler_fires_due_messages_once_and_cancels_by_name() {
        let at = Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap();
        let scheduler = MemoryScheduler::new();
        scheduler
            .schedule("one", at, &message("1", "g"))
            .await
            .unwrap();
        scheduler
            .schedule("two", at + chrono::Duration::minutes(1), &message("2", "g"))
            .await
            .unwrap();
        scheduler
            .schedule("three", at, &message("3", "g"))
            .await
            .unwrap();
        scheduler.cancel("three").await.unwrap();
        assert!(
            scheduler
                .take_due(at - chrono::Duration::seconds(1))
                .is_empty()
        );
        assert_eq!(scheduler.take_due(at)[0].body, "1");
        assert!(scheduler.take_due(at).is_empty());
        assert_eq!(scheduler.pending().len(), 1);
    }
}
