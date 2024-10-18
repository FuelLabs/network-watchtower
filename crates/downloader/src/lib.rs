//! Example of subscribing and listening for specific contract events by `WebSocket` subscription.

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
use serde::Deserialize;

use fuel_block_committer_encoding::{
    blob::{
        self,
        Blob,
        Header,
    },
    bundle::{
        self,
        Bundle,
    },
};

use sha2::{
    Digest,
    Sha256,
};

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
) -> anyhow::Result<Vec<Vec<u8>>> {
    let mut hashes = Vec::new();
    for tx in block.transactions.txns() {
        if tx.from == *target_contract && tx.transaction_type == Some(3) {
            if let Some(bvh) = tx.blob_versioned_hashes.as_ref() {
                hashes.extend(bvh.into_iter().map(|h| h.to_vec()));
            }
        }
    }
    Ok(hashes)
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
    /// Cached block at `current_block + 1`.
    /// Used to avoid fetching the same block multiple times.
    peek_block: Option<Block>,
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
            peek_block: None,
        })
    }

    async fn download_block(&mut self, number: u64) -> anyhow::Result<Block> {
        if self.current_block + 1 == number {
            if let Some(block) = self.peek_block.clone() {
                return Ok(block);
            }
        }

        let block = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Number(number), true)
            .await?
            .ok_or_else(|| anyhow::anyhow!("block not found"))?;

        if self.current_block + 1 == number {
            self.peek_block = Some(block.clone());
        }

        Ok(block)
    }

    async fn get_sidecars_for_current_block(
        &mut self,
    ) -> anyhow::Result<Option<BlobSidecarResponse>> {
        let next_block = self.download_block(self.current_block + 1).await?;

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

    /// Fetches bundles and advances to the next block if successful.
    pub async fn next_block_bundles(&mut self) -> anyhow::Result<Vec<(Header, Bundle)>> {
        let this_block = self.download_block(self.current_block).await?;
        let blob_ids = get_block_tx_blobs(&this_block, &self.target_contract)?;

        let sidecars = self.get_sidecars_for_current_block().await?;
        let mut result = Vec::new();
        if let Some(sidecars) = sidecars {
            result.extend(get_blobs_from_beacon_response(sidecars, blob_ids)?);
        }

        self.current_block += 1;
        Ok(result)
    }
}
