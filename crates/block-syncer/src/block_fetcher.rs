use fuel_core_client::client::{
    pagination::{
        PageDirection,
        PaginationRequest,
    },
    FuelClient,
};
use fuel_core_client_ext::ClientExt;
use fuel_core_compression::VersionedCompressedBlock;
use fuel_core_types::{
    blockchain::block::Block,
    fuel_types::BlockHeight,
};
use itertools::Itertools;
use std::ops::Range;

pub struct BlockFetcher {
    client: FuelClient,
}

impl BlockFetcher {
    pub fn new(url: impl AsRef<str>) -> anyhow::Result<Self> {
        let client = FuelClient::new(url)?;
        Ok(Self { client })
    }
}

impl BlockFetcher {
    async fn last_height(&self) -> anyhow::Result<BlockHeight> {
        let chain_info = self.client.chain_info().await?;
        let height = chain_info.latest_block.header.height.into();

        Ok(height)
    }

    async fn block_for(&self, range: Range<u32>) -> anyhow::Result<Vec<Block>> {
        if range.is_empty() {
            return Ok(vec![]);
        }

        let start = range.start.saturating_sub(1);
        let size = range.len() as i32;

        let request = PaginationRequest {
            cursor: Some(start.to_string()),
            results: size,
            direction: PageDirection::Forward,
        };
        let response = self.client.full_blocks(request).await?;
        let blocks = response
            .results
            .into_iter()
            .map(TryInto::try_into)
            .try_collect()?;
        Ok(blocks)
    }

    async fn compressed_block_for(
        &self,
        range: Range<u32>,
    ) -> anyhow::Result<Vec<VersionedCompressedBlock>> {
        if range.is_empty() {
            return Ok(vec![]);
        }

        let futures = range
            .into_iter()
            .map(|i| {
                let block_height: BlockHeight = i.into();
                self.client.da_compressed_block(block_height)
            })
            .collect_vec();

        let compressed_blocks = futures::future::try_join_all(futures).await?;

        let compressed_blocks = compressed_blocks
            .into_iter()
            .flatten()
            .map(|block| postcard::from_bytes(&block))
            .try_collect()?;

        Ok(compressed_blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn testnet_works() {
        let syncer = BlockFetcher::new("https://testnet.fuel.network")
            .expect("Should connect to the testnet network");
        // Given
        const START: u32 = 77;
        const END: u32 = 110;

        // When
        let result = syncer.block_for(START..END).await;

        // Then
        let blocks = result.expect("Should get blocks");
        assert_eq!(blocks.len(), (END - START) as usize);

        for i in START..END {
            let block: &Block = &blocks[i.saturating_sub(START) as usize];
            assert_eq!(*block.header().height(), i.into());
        }
    }

    #[tokio::test]
    async fn compressed_blocks_works() {
        let syncer = BlockFetcher::new("http://127.0.0.1:4000")
            .expect("Should connect to local network");

        // Given
        const TO_SYNC: u32 = 40;
        let end: u32 = syncer.last_height().await.unwrap().into();
        let start = end.saturating_sub(TO_SYNC);

        // When
        let result = syncer.compressed_block_for(start..end).await;

        // Then
        let blocks = result.expect("Should get blocks");
        assert_eq!(blocks.len(), (end - start) as usize);

        for i in start..end {
            let block: &VersionedCompressedBlock =
                &blocks[i.saturating_sub(start) as usize];

            let VersionedCompressedBlock::V0(block) = block;

            assert_eq!(*block.header.height(), i.into());
        }
    }
}
