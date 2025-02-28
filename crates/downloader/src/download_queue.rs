use std::{
    collections::BTreeMap,
    future::Future,
    time::Duration,
};

use alloy::{
    eips::BlockNumberOrTag,
    providers::{
        Provider,
        RootProvider,
    },
    rpc::types::Block,
    transports::{
        http::Http,
        RpcError,
        TransportErrorKind,
    },
};
use futures_util::Stream;
use tokio::{
    sync::mpsc,
    task::JoinSet,
};
use tokio_stream::wrappers::ReceiverStream;

#[derive(Debug)]
pub enum GetBlockError {
    RateLimited,
    Other(RpcError<TransportErrorKind>),
}

pub trait GetBlock {
    fn get_block(
        &self,
        block_number: u64,
    ) -> impl Future<Output = Result<Option<Block>, GetBlockError>> + Send;
}

impl GetBlock for RootProvider<Http<reqwest::Client>> {
    async fn get_block(&self, block_number: u64) -> Result<Option<Block>, GetBlockError> {
        self.get_block_by_number(BlockNumberOrTag::Number(block_number), true)
            .await
            .map_err(|err| match err {
                RpcError::ErrorResp(resp) if resp.code == 429 => {
                    GetBlockError::RateLimited
                }
                other => GetBlockError::Other(other),
            })
    }
}

enum Mode {
    /// Batch-download blocks until we reach the end of the chain
    Batch,
    /// Poll for new blocks
    Poll,
}
impl Mode {
    /// How many blocks to download concurrently
    fn concurrency(&self) -> usize {
        match self {
            Mode::Batch => 8,
            Mode::Poll => 1,
        }
    }
}

pub struct DownloadQueue<P> {
    provider: P,
    next_da_height: u64,
    pending: JoinSet<Result<Option<Block>, GetBlockError>>,
    completed: BTreeMap<u64, Block>,
}

impl<P> DownloadQueue<P>
where
    P: GetBlock + Clone + Send + 'static,
{
    pub fn start_from(provider: P, da_height: u64) -> Self {
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
        let mut next_to_emit = self.next_da_height;
        let mut mode = Mode::Batch;
        const DA_BLOCK_TIME_MILLIS: u64 = 12_000;

        loop {
            // Queue new downloads up to the concurrency limit
            while self.pending.len() + self.completed.len() < mode.concurrency() {
                let provider = self.provider.clone();
                self.pending
                    .spawn(async move { provider.get_block(self.next_da_height).await });
                self.next_da_height += 1;
            }

            // Wait for the next block to complete
            if let Some(block) = self.pending.join_next().await {
                match block.expect("Failed to join block download task") {
                    Ok(Some(block)) => {
                        self.completed.insert(block.header.number, block);
                    }
                    Ok(None) => {
                        if matches!(mode, Mode::Batch) {
                            tracing::info!(
                                "Latest block reached, moving to polling mode"
                            );
                            mode = Mode::Poll;
                        } else {
                            self.next_da_height = next_to_emit;
                            if self.pending.is_empty() {
                                tracing::trace!(
                                    "Block is not yet available, trying again later."
                                );
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    DA_BLOCK_TIME_MILLIS,
                                ))
                                .await;
                            }
                            continue;
                        }
                    }
                    Err(GetBlockError::RateLimited) => {
                        tracing::warn!("Hit rate limit, waiting 5s");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    Err(GetBlockError::Other(err)) => {
                        return Err(err.into());
                    }
                }
            }

            // Emit all completed blocks from the beginning
            while let Some(head) = self.completed.first_entry() {
                if head.key() == &next_to_emit {
                    let block = head.remove();
                    tx.send(Ok(block)).await?;
                    next_to_emit += 1;
                } else {
                    break;
                }
            }
        }
    }

    pub fn stream(self) -> impl Stream<Item = anyhow::Result<Block>> {
        let (tx, rx) = mpsc::channel(1);
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

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::Duration,
    };

    use alloy::{
        primitives::map::HashMap,
        rpc::types::{
            Block,
            Header,
        },
    };
    use futures_util::StreamExt;
    use tokio::{
        sync::{
            mpsc,
            Mutex,
        },
        time::{
            self,
            Instant,
        },
    };

    use super::{
        DownloadQueue,
        GetBlock,
        GetBlockError,
    };

    #[derive(Clone, Default)]
    pub struct MockProvider {
        /// Holds pre-set response data
        data: Arc<Mutex<HashMap<u64, Result<Block, GetBlockError>>>>,
    }
    impl GetBlock for MockProvider {
        async fn get_block(
            &self,
            block_number: u64,
        ) -> Result<Option<Block>, GetBlockError> {
            if let Some(block_result) = self.data.lock().await.remove(&block_number) {
                block_result.map(Some)
            } else {
                Ok(None)
            }
        }
    }

    fn mock_block(number: u64) -> Block {
        Block {
            header: Header {
                number,
                ..Default::default()
            },
            uncles: Default::default(),
            transactions: Default::default(),
            size: Default::default(),
            withdrawals: Default::default(),
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn switches_to_polling_when_end_is_reached() {
        let provider = MockProvider::default();

        let mut dlq = DownloadQueue::start_from(provider.clone(), 0).stream();

        // First go through a batch of blocks. This is done without delay.
        let start_time = Instant::now();

        let count = 100;
        for i in 0..count {
            provider.data.lock().await.insert(i, Ok(mock_block(i)));
        }
        for i in 0..count {
            let block = dlq
                .next()
                .await
                .expect("Download stream ended unexpectedly")
                .expect("Block download failed");
            assert_eq!(block.header.number, i);
        }
        assert_eq!(start_time.elapsed(), Duration::new(0, 0));

        // Now the provider will return None, so we'll transfer to the polling mode.
        let (tx, mut rx) = mpsc::channel(1);
        let _task = tokio::spawn(async move {
            loop {
                let block = dlq
                    .next()
                    .await
                    .expect("Download stream ended unexpectedly")
                    .expect("Block download failed");
                let _ = tx.send(block.header.number).await;
            }
        });

        for i in 0..10 {
            let blocknum = count + i;
            provider
                .data
                .lock()
                .await
                .insert(blocknum, Ok(mock_block(blocknum)));

            assert_eq!(rx.recv().await.unwrap(), blocknum);
            time::advance(Duration::new(12, 0)).await;
        }
    }
}
