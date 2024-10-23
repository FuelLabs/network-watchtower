//! Example of subscribing and listening for specific contract events by `WebSocket` subscription.
#![deny(clippy::cast_possible_truncation)]
#![deny(unused_crate_dependencies)]
#![deny(warnings)]

use alloy::{
    primitives::Address,
    providers::{
        Provider,
        ProviderBuilder,
        RootProvider,
    },
    rpc::types::{
        Block,
        BlockNumberOrTag,
    },
    transports::http::Http,
};
use fuel_block_committer_encoding::{
    blob::{
        self,
        Blob,
        Header,
        HeaderV1,
    },
    bundle::{
        self,
        Bundle,
    },
};
use fuel_core_compression::VersionedCompressedBlock;
use fuel_core_types::fuel_types::BlockHeight;
use futures_util::Stream;
use itertools::Itertools;
use reqwest::Url;
use serde::Deserialize;
use sha2::{
    Digest,
    Sha256,
};
use std::collections::{
    BTreeMap,
    HashMap,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

mod config;

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
    /// Ethereum provider client.
    provider: RootProvider<Http<reqwest::Client>>,
    /// HTTP client for downloading blobs.
    http: reqwest::Client,
    /// API URL for the beacon.
    beacon_url: Url,
    /// Contract to monitor for new blobs.
    target_contract: Address,
    /// Current block number.
    current_block: u64,
}

impl Downloader {
    pub fn new(config: Config) -> Self {
        let rpc_url = config.ethereum_rpc_url;
        let provider = ProviderBuilder::new().on_http(rpc_url);
        let http = reqwest::Client::new();
        let target_contract = Address::from(*config.blob_contract);
        Self {
            provider,
            http,
            beacon_url: config.beacon_rpc_url,
            target_contract,
            current_block: config.start_block,
        }
    }

    async fn download_block(&self, number: u64) -> anyhow::Result<Option<Block>> {
        Ok(self
            .provider
            .get_block_by_number(BlockNumberOrTag::Number(number), true)
            .await?)
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

    async fn stream_inner(
        mut self,
        tx: mpsc::Sender<Result<VersionedCompressedBlock, anyhow::Error>>,
    ) -> anyhow::Result<()> {
        let mut bundle_buffer_by_id: HashMap<u32, Vec<(HeaderV1, Blob)>> = HashMap::new();
        loop {
            tracing::trace!("Trying to processing block: {}", self.current_block);
            // TODO: reuse next_block from the previous iteration to avoid an extra api call
            let Some(this_block) = self.download_block(self.current_block).await? else {
                tracing::trace!("Block is not yet available, try again later.");
                tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                continue;
            };

            let Some(next_block) = self.download_block(self.current_block + 1).await?
            else {
                tracing::trace!("Block is not yet available, try again later.");
                tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                continue;
            };

            tracing::info!("Processing block: {}", self.current_block);
            self.current_block += 1;

            let blob_ids = get_block_tx_blobs(&this_block, &self.target_contract);
            if let Some(sidecars) = self.get_sidecars_from_next_block(&next_block).await?
            {
                let blobs = get_blobs_from_beacon_response(sidecars, blob_ids)?;
                'blobs: for blob in blobs {
                    let header = blob::Decoder::default().read_header(&blob)?;
                    let Header::V1(header) = header;
                    let bundle_id = header.bundle_id;
                    // Insert into the buffer of this bundle, keeping the blobs inside it sorted
                    let blobs = bundle_buffer_by_id.entry(bundle_id).or_default();
                    match blobs
                        .binary_search_by_key(&header.idx, |(header, _)| header.idx)
                    {
                        Ok(_) => {
                            anyhow::bail!("Duplicate blob with index {}", header.idx)
                        }
                        Err(idx) => {
                            tracing::info!(
                                "Inserted blob for the block {}, bundle {}, index {}, last {}",
                                self.current_block,
                                header.bundle_id,
                                header.idx,
                                header.is_last,
                            );
                            blobs.insert(idx, (header, blob));
                        }
                    }
                    // Check if the bundle is complete
                    if let Some((last_header, _)) = blobs.last() {
                        if !last_header.is_last {
                            // Not complete yet
                            continue 'blobs;
                        }
                    }
                    let mut expected_idx = 0;
                    for (header, _) in blobs {
                        if header.idx != expected_idx {
                            // Not complete yet
                            continue 'blobs;
                        }
                        expected_idx += 1;
                    }

                    // Now we do have a complete bundle
                    let blobs = bundle_buffer_by_id
                        .remove(&bundle_id)
                        .expect("This was inserted above");
                    let blobs: Vec<Blob> =
                        blobs.into_iter().map(|(_, blob)| blob).collect();

                    let bundle_bytes = blob::Decoder::default().decode(&blobs)?;
                    let bundle: Bundle =
                        bundle::Decoder::default().decode(&bundle_bytes)?;
                    let Bundle::V1(bundle) = bundle;

                    let blocks: BTreeMap<BlockHeight, VersionedCompressedBlock> = bundle
                        .blocks
                        .into_iter()
                        .map(|block| {
                            let block: VersionedCompressedBlock =
                                postcard::from_bytes(&block)
                                    .map_err(anyhow::Error::from)?;

                            Ok::<_, anyhow::Error>((*block.height(), block))
                        })
                        .try_collect()?;

                    for (block_height, block) in blocks {
                        tracing::info!(
                            "Sending block {} from bundle {}",
                            block_height,
                            bundle_id
                        );
                        tx.send(Ok(block)).await?;
                    }
                }
            }
        }
    }
}
