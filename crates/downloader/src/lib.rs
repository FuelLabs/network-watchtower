//! Example of subscribing and listening for specific contract events by `WebSocket` subscription.

use alloy::{
    primitives::Address,
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::types::{Block, BlockNumberOrTag},
    transports::http::Http,
};
use futures_util::{Stream};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use fuel_block_committer_encoding::{
    blob::{self, Blob, Header},
    bundle::{self, Bundle},
};

use sha2::{Digest, Sha256};

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
                hashes.extend(bvh.into_iter().map(|h| h.to_vec()));
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
) -> anyhow::Result<Vec<(Header, Bundle)>> {
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

            let blob_decoder = blob::Decoder::default();
            let blob_header = blob_decoder.read_header(&blob)?;

            let bundle_bytes = blob_decoder.decode(&[blob])?;
            let bundle: Bundle = bundle::Decoder::default().decode(&bundle_bytes)?;

            result.push((blob_header, bundle));
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
    beacon_url: String,
    /// Contract to monitor for new blobs.
    target_contract: Address,
    /// Current block number.
    current_block: u64,
}
impl Downloader {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let rpc_url = config.ethereum_rpc_url.parse()?;
        let provider = ProviderBuilder::new().on_http(rpc_url);
        let http = reqwest::Client::new();
        let target_contract: Address = config.blob_contract.parse()?;
        Ok(Self {
            provider,
            http,
            beacon_url: config.beacon_rpc_url,
            target_contract,
            current_block: config.start_block,
        })
    }

    async fn download_block(&self, number: u64) -> anyhow::Result<Option<Block>> {
        Ok(self
            .provider
            .get_block_by_number(BlockNumberOrTag::Number(number), true)
            .await?
        )
    }

    /// Gets sidecars for a block, using the next block's parent beacon block root.
    async fn get_sidecars_from_next_block(
        &self, next_block: &Block,
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

    pub fn stream<'a>(mut self) -> impl Stream<Item = anyhow::Result<(Header, Bundle)>> + 'a {
        let (tx, rx) = mpsc::channel(1);

        tokio::spawn(async move {
            loop {                
                let this_block = match self.download_block(self.current_block).await {
                    Ok(Some(block)) => block,
                    Ok(None) => {
                        // Block is not yet available, try again later.
                        log::trace!("Block is not yet available, try again later.");
                        tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                        continue;
                    },
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                };

                let next_block = match self.download_block(self.current_block + 1).await {
                    Ok(Some(block)) => block,
                    Ok(None) => {
                        // Block is not yet available, try again later.
                        log::trace!("Block is not yet available, try again later.");
                        tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                        continue;
                    },
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                };

                self.current_block += 1;
                log::trace!("Processing block: {}", self.current_block);

                let blob_ids = get_block_tx_blobs(&this_block, &self.target_contract);
                match self.get_sidecars_from_next_block(&next_block).await {
                    Ok(Some(sidecars)) => {
                        match get_blobs_from_beacon_response(sidecars, blob_ids) {
                            Ok(items) => {
                                for item in items {
                                    if tx.send(Ok(item)).await.is_err() {
                                        return;
                                    }
                                }
                            },
                            Err(e) => {
                                if tx.send(Err(e)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    },
                    Ok(None) => {},
                    Err(e) => {
                        if tx.send(Err(e)).await.is_err() {
                            return;
                        }
                    },
                }
            }
        });

        ReceiverStream::new(rx)
    }
}
