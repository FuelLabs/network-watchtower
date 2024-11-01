#![deny(clippy::cast_possible_truncation)]
#![deny(unused_crate_dependencies)]
// #![deny(warnings)]

use std::pin::Pin;

use alloy::{
    primitives::Address,
    providers::ProviderBuilder,
    rpc::types::Block,
};
use block_buffer::BlockBuffer;
use bundle_buffer::{
    IncompleteBundleBuffers,
    Inserted,
};
use download_queue::DownloadQueue;
use fuel_block_committer_encoding::blob::Blob;
use fuel_core_compression::VersionedCompressedBlock;
use fuel_core_types::fuel_types::BlockHeight;
use futures_util::{
    Stream,
    StreamExt,
};
use reqwest::Url;
use serde::Deserialize;
use sha2::{
    Digest,
    Sha256,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

mod block_buffer;
mod bundle_buffer;
mod config;
mod download_queue;

pub use config::Config;

#[derive(Debug, Clone, Deserialize)]
struct BlobSidecarResponse {
    data: Vec<BlobData>,
}

#[derive(Debug, Clone, Deserialize)]
struct BlobData {
    blob: String,
    kzg_commitment: String,
}

/// Returns blob hashes of the target contract from the block.
fn get_block_tx_blobs(
    block: &alloy::rpc::types::Block,
    target_contract: &Address,
) -> Vec<Vec<u8>> {
    let mut hashes = Vec::new();
    for tx in block.transactions.txns() {
        if tx.from == *target_contract && tx.transaction_type == Some(3) {
            if let Some(bvh) = tx.blob_versioned_hashes.as_ref() {
                hashes.extend(bvh.iter().map(|h| h.to_vec()));
            }
        }
    }
    hashes
}

/// Returns blobs from the beacon response,
/// given the hashes of the blobs we are interested in.
fn get_blobs_from_beacon_response(
    body: BlobSidecarResponse,
    hashes: Vec<Vec<u8>>,
) -> anyhow::Result<Vec<Blob>> {
    let mut result = Vec::new();
    for item in body.data.clone().into_iter() {
        // Compute the sha256 of the kzg commitment, which is used to identify the blob
        let kzg_commitment = hex::decode(item.kzg_commitment.trim_start_matches("0x"))?;
        let mut kzg_hash = Sha256::new();
        kzg_hash.update(kzg_commitment);
        let digest = kzg_hash.finalize();
        let mut blob_versioned_hash_hex = vec![0x01];
        blob_versioned_hash_hex.extend(&digest[1..]);

        // Found a matching blob
        if hashes.contains(&blob_versioned_hash_hex.to_vec()) {
            let hex_str = item.blob.trim_start_matches("0x");
            let blob_raw_data = hex::decode(hex_str)?;

            let blob: Blob = blob_raw_data
                .into_boxed_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid Blob bytes"))?;

            result.push(blob);
        }
    }

    Ok(result)
}

pub struct Downloader {
    /// HTTP client for downloading blobs.
    http: reqwest::Client,
    /// API URL for the beacon.
    beacon_url: Url,
    /// Contract to monitor for new blobs.
    target_contract: Address,
    /// Stream of blocks downloaded from the L1.
    download_stream:
        Pin<Box<dyn Stream<Item = anyhow::Result<Block>> + Send + Sync + 'static>>,
    /// Next Fuel block to emit.
    next_fuel_block: BlockHeight,
    /// Incomplete bundle buffers.
    bundle_buffers: IncompleteBundleBuffers,
    /// Outgoing block buffer.
    block_buffer: BlockBuffer,
}

impl Downloader {
    pub fn new(config: Config) -> Self {
        let rpc_url = config.ethereum_rpc_url;
        let provider = ProviderBuilder::new().on_http(rpc_url);
        let http = reqwest::Client::new();
        let target_contract = Address::from(*config.blob_contract);
        Self {
            http,
            beacon_url: config.beacon_rpc_url,
            target_contract,
            download_stream: Box::pin(
                DownloadQueue::start_from(provider, config.da_start_block).stream(),
            ),
            next_fuel_block: config.next_fuel_block,
            bundle_buffers: IncompleteBundleBuffers::default(),
            block_buffer: BlockBuffer::default(),
        }
    }

    /// Gets sidecars for a block, using the next block's parent beacon block root.
    async fn get_sidecars_from_next_block(
        &self,
        next_block: &Block,
    ) -> anyhow::Result<Option<BlobSidecarResponse>> {
        let Some(next_pbbr) = next_block.header.parent_beacon_block_root else {
            // No parent beacon block root, which means no blobs either.
            return Ok(None);
        };

        let url = format!(
            "{}/eth/v1/beacon/blob_sidecars/{}",
            self.beacon_url, next_pbbr
        );

        Ok(Some(self.http.get(&url).send().await?.json().await?))
    }

    /// Fetch blobs from the beacon and process them, emitting blocks as they are completed.
    async fn process_block(
        &mut self,
        this_block: &Block,
        next_block: &Block,
        tx: &mpsc::Sender<Result<VersionedCompressedBlock, anyhow::Error>>,
    ) -> Result<(), anyhow::Error> {
        let blob_ids = get_block_tx_blobs(this_block, &self.target_contract);
        let Some(sidecars) = self.get_sidecars_from_next_block(next_block).await? else {
            return Ok(());
        };
        let blobs = get_blobs_from_beacon_response(sidecars, blob_ids)?;
        for blob in blobs {
            let Inserted::Complete(bundle) = self.bundle_buffers.insert(blob)? else {
                continue;
            };

            for block in bundle.blocks {
                let block: VersionedCompressedBlock =
                    postcard::from_bytes(&block).map_err(anyhow::Error::from)?;
                self.block_buffer.push(block);
            }

            self.block_buffer.prune_below(self.next_fuel_block);

            while let Some(block) = self.block_buffer.pop_height(self.next_fuel_block) {
                tracing::debug!("Sending block {}", block.height());
                tx.send(Ok(block)).await?;
                self.next_fuel_block = self
                    .next_fuel_block
                    .succ()
                    .ok_or_else(|| anyhow::anyhow!("Out of Fuel block numbers"))?;
            }
        }
        Ok(())
    }

    async fn stream_inner(
        mut self,
        tx: mpsc::Sender<Result<VersionedCompressedBlock, anyhow::Error>>,
    ) -> anyhow::Result<()> {
        let mut this_block = self
            .download_stream
            .next()
            .await
            .expect("Download stream ended unexpectedly")?;
        loop {
            let next_block = self
                .download_stream
                .next()
                .await
                .expect("Download stream ended unexpectedly")?;
            self.process_block(&this_block, &next_block, &tx).await?;
            this_block = next_block;
        }
    }

    pub fn stream(self) -> impl Stream<Item = anyhow::Result<VersionedCompressedBlock>> {
        let (tx, rx) = mpsc::channel(1024);
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
