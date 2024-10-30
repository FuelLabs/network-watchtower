use std::collections::BTreeMap;

use alloy::{
    eips::BlockNumberOrTag,
    providers::{
        Provider,
        RootProvider,
    },
    rpc::types::Block,
    transports::http::Http,
};
use futures_util::Stream;
use tokio::{
    sync::mpsc,
    task::JoinSet,
};
use tokio_stream::wrappers::ReceiverStream;

/// How many blocks to download at once.
const CONCURRENCY: usize = 8;

pub struct DownloadQueue {
    provider: RootProvider<Http<reqwest::Client>>,
    next_da_height: u64,
    pending: JoinSet<anyhow::Result<Option<Block>>>,
    completed: BTreeMap<u64, Block>,
}

impl DownloadQueue {
    pub fn start_from(
        provider: RootProvider<Http<reqwest::Client>>,
        da_height: u64,
    ) -> Self {
        Self {
            provider,
            next_da_height: da_height,
            pending: JoinSet::new(),
            completed: BTreeMap::new(),
        }
    }

    async fn stream_inner(
        mut self,
        tx: mpsc::Sender<Result<Block, anyhow::Error>>,
    ) -> anyhow::Result<()> {
        let next_to_emit = self.next_da_height;

        // First, batch-download blocks until we reach the end of the chain
        loop {
            // Queue new downloads up to the concurrency limit
            while self.pending.len() + self.completed.len() < CONCURRENCY {
                let provider = self.provider.clone();
                self.pending.spawn(async move {
                    provider
                        .get_block_by_number(
                            BlockNumberOrTag::Number(self.next_da_height),
                            true,
                        )
                        .await
                        .map_err(anyhow::Error::from)
                });
                self.next_da_height += 1;
            }

            // Wait for the next block to complete
            if let Some(block) = self.pending.join_next().await {
                let block = block.expect("Failed to join block download task")?;
                if let Some(block) = block {
                    self.completed.insert(block.header.number, block);
                } else {
                    break; // Reached end of chain
                }
            }

            // Emit all completed blocks from the beginning
            while let Some(head) = self.completed.first_entry() {
                if head.key() == &next_to_emit {
                    let block = head.remove();
                    tx.send(Ok(block)).await?;
                } else {
                    break;
                }
            }
        }

        // Poll blocks
        loop {
            let Some(block) = self
                .provider
                .get_block_by_number(BlockNumberOrTag::Number(self.next_da_height), true)
                .await?
            else {
                tracing::trace!("Block is not yet available, try again later.");
                tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                continue;
            };
            tx.send(Ok(block)).await?;
            self.next_da_height += 1;
        }
    }

    pub fn stream(self) -> impl Stream<Item = anyhow::Result<Block>> {
        let (tx, rx) = mpsc::channel(8);
        let stream = self.stream_inner(tx.clone());

        tokio::spawn(async move {
            match stream.await {
                Ok(_) => unreachable!("Downloader stream ended unexpectedly"),
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                }
            }
        });

        ReceiverStream::new(rx)
    }
}
