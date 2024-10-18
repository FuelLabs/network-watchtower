use client_ext::ClientExt;
use fuel_core::types::blockchain::block::Block;
use fuel_core_client::client::{
    pagination::{
        PageDirection,
        PaginationRequest,
    },
    FuelClient,
};
use itertools::Itertools;
use std::ops::Range;

pub struct BlockSyncer {
    client: FuelClient,
}

impl BlockSyncer {
    pub fn new(url: impl AsRef<str>) -> anyhow::Result<Self> {
        let client = FuelClient::new(url)?;
        Ok(Self { client })
    }
}

impl BlockSyncer {
    async fn block_for(&self, range: Range<u32>) -> anyhow::Result<Vec<Block>> {
        if range.is_empty() {
            return Ok(vec![]);
        }

        let start = range.start.saturating_sub(1);
        let size = range.end.saturating_sub(range.start);

        let request = PaginationRequest {
            cursor: Some(start.to_string()),
            results: size as i32,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn testnet_works() {
        let syncer = BlockSyncer::new("https://testnet.fuel.network")
            .expect("Should connect to the beta 5 network");
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
}
