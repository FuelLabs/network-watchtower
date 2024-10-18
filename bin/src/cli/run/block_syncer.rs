use client_ext::ClientExt;
use fuel_core::types::blockchain::block::Block;
use fuel_core_client::client::{
    pagination::{
        PageDirection,
        PaginationRequest,
    },
    FuelClient,
};
use url::Url;

pub struct BlockSyncer {
    client: FuelClient,
}

impl BlockSyncer {
    pub fn new(url: Url) -> anyhow::Result<Self> {
        let client = FuelClient::new(url.as_str())?;
        Ok(Self { client })
    }
}

impl BlockSyncer {
    async fn get_full_block(&self) -> anyhow::Result<Block> {
        let request = PaginationRequest {
            cursor: None,
            results: 40,
            direction: PageDirection::Forward,
        };
        let response = self.client.full_blocks(request).await?;
        let _response = response.results;
        todo!()
    }
}
