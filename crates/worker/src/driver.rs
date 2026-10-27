use std::sync::Arc;
use std::time::Duration as StdDuration;

use sanchoris_core::jobs::Envelope;
use sanchoris_core::local::{MemoryQueue, MemoryScheduler};
use sanchoris_core::ports::Queue;

use crate::error::{Result, WorkerError};
use crate::{PERIODIC_JOBS, Worker};

pub struct LocalDriver {
    pub worker: Arc<Worker>,
    pub queue: Arc<MemoryQueue>,
    pub scheduler: Arc<MemoryScheduler>,
}

impl LocalDriver {
    pub fn new(
        worker: Arc<Worker>,
        queue: Arc<MemoryQueue>,
        scheduler: Arc<MemoryScheduler>,
    ) -> Self {
        Self {
            worker,
            queue,
            scheduler,
        }
    }

    pub async fn fire_due(&self) -> Result<usize> {
        let due = self.scheduler.take_due(self.worker.now());
        for message in &due {
            self.queue.send(message).await?;
        }
        Ok(due.len())
    }

    pub async fn run_periodic(&self) -> Result<usize> {
        let mut progressed = 0;
        for job in PERIODIC_JOBS {
            progressed += self.worker.handle(&Envelope::new(job)).await?;
        }
        Ok(progressed)
    }

    pub async fn handle_next(&self) -> Result<Option<usize>> {
        let Some(message) = self.queue.receive(1).await?.into_iter().next() else {
            return Ok(None);
        };
        match self.worker.handle_body(&message.body).await {
            Ok(progressed) => {
                self.queue.delete(&message.receipt).await?;
                Ok(Some(progressed.max(1)))
            }
            Err(error) => {
                self.queue.release(&message.receipt).await?;
                Err(error)
            }
        }
    }

    pub async fn drain(&self) -> Result<usize> {
        let mut handled = 0;
        let mut failure = None;
        loop {
            let messages = self.queue.receive(usize::MAX).await?;
            let mut progressed = false;
            for message in messages {
                match self.worker.handle_body(&message.body).await {
                    Ok(count) => {
                        self.queue.delete(&message.receipt).await?;
                        handled += count.max(1);
                        progressed = true;
                    }
                    Err(error) => {
                        self.queue.release(&message.receipt).await?;
                        failure.get_or_insert(error);
                    }
                }
            }
            if !progressed {
                break;
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(handled),
        }
    }

    pub async fn round(&self) -> Result<usize> {
        let mut progressed = self.fire_due().await?;
        progressed += self.drain().await?;
        progressed += self.run_periodic().await?;
        progressed += self.drain().await?;
        Ok(progressed)
    }

    pub async fn settle(&self) -> Result<usize> {
        let mut total = 0;
        loop {
            let progressed = self.round().await?;
            if progressed == 0 {
                return Ok(total);
            }
            total += progressed;
        }
    }

    pub async fn run(
        self: Arc<Self>,
        interval: StdDuration,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut ticker = tokio::time::interval(interval);
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = shutdown.changed() => return,
            }
            if let Err(error) = self.round().await
                && !matches!(error, WorkerError::Injected(_))
            {
                eprintln!("sanchoris worker: {error}");
            }
        }
    }
}
