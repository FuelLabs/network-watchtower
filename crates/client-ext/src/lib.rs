use cynic::QueryBuilder;
use fuel_core_client::client::{
    pagination::{
        PaginatedResult,
        PaginationRequest,
    },
    schema::{
        block::{
            BlockByHeightArgs,
            Consensus,
            Header,
        },
        primitives::TransactionId,
        schema,
        tx::TransactionStatus,
        BlockId,
        ConnectionArgs,
        HexString,
        PageInfo,
    },
    FuelClient,
};
use fuel_core_types::fuel_crypto::PublicKey;

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./target/schema.sdl",
    graphql_type = "Query",
    variables = "ConnectionArgs"
)]
pub struct FullBlocksQuery {
    #[arguments(after: $after, before: $before, first: $first, last: $last)]
    pub blocks: FullBlockConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./target/schema.sdl", graphql_type = "BlockConnection")]
pub struct FullBlockConnection {
    pub edges: Vec<FullBlockEdge>,
    pub page_info: PageInfo,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./target/schema.sdl", graphql_type = "BlockEdge")]
pub struct FullBlockEdge {
    pub cursor: String,
    pub node: FullBlock,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./target/schema.sdl",
    graphql_type = "Query",
    variables = "BlockByHeightArgs"
)]
pub struct FullBlockByHeightQuery {
    #[arguments(height: $height)]
    pub block: Option<FullBlock>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./target/schema.sdl", graphql_type = "Block")]
pub struct FullBlock {
    pub id: BlockId,
    pub header: Header,
    pub consensus: Consensus,
    pub transactions: Vec<OpaqueTransaction>,
}

impl FullBlock {
    /// Returns the block producer public key, if any.
    pub fn block_producer(&self) -> Option<PublicKey> {
        let message = self.header.id.clone().into_message();
        match &self.consensus {
            Consensus::Genesis(_) => Some(Default::default()),
            Consensus::PoAConsensus(poa) => {
                let signature = poa.signature.clone().into_signature();
                let producer_pub_key = signature.recover(&message);
                producer_pub_key.ok()
            }
            Consensus::Unknown => None,
        }
    }
}

impl From<FullBlockConnection> for PaginatedResult<FullBlock, String> {
    fn from(conn: FullBlockConnection) -> Self {
        PaginatedResult {
            cursor: conn.page_info.end_cursor,
            has_next_page: conn.page_info.has_next_page,
            has_previous_page: conn.page_info.has_previous_page,
            results: conn.edges.into_iter().map(|e| e.node).collect(),
        }
    }
}

#[derive(cynic::QueryFragment, Clone, Debug)]
#[cynic(schema_path = "./target/schema.sdl", graphql_type = "Transaction")]
pub struct OpaqueTransaction {
    pub id: TransactionId,
    pub raw_payload: HexString,
    pub status: Option<TransactionStatus>,
}

#[async_trait::async_trait]
pub trait ClientExt {
    async fn full_blocks(
        &self,
        request: PaginationRequest<String>,
    ) -> std::io::Result<PaginatedResult<FullBlock, String>>;
}

#[async_trait::async_trait]
impl ClientExt for FuelClient {
    async fn full_blocks(
        &self,
        request: PaginationRequest<String>,
    ) -> std::io::Result<PaginatedResult<FullBlock, String>> {
        let query = FullBlocksQuery::build(request.into());
        let blocks = self.query(query).await?.blocks.into();
        Ok(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuel_core_client::client::pagination::PageDirection;

    #[tokio::test]
    async fn testnet_works() {
        let client = FuelClient::new("http://127.0.0.1:4000")
            .expect("Should connect to the beta 5 network");

        let producer_before_4 = client.block_by_height(9704464.into()).await.unwrap().unwrap().block_producer;
        let producer_before_3 = client.block_by_height(9704465.into()).await.unwrap().unwrap().block_producer;
        let producer_before_2 = client.block_by_height(9704466.into()).await.unwrap().unwrap().block_producer;
        let producer_before_1 = client.block_by_height(9704467.into()).await.unwrap().unwrap().block_producer;
        let producer_before = client.block_by_height(9704468.into()).await.unwrap().unwrap().block_producer;
        let producer_after = client.block_by_height(9704469.into()).await.unwrap().unwrap().block_producer;
        let producer_after_1 = client.block_by_height(9704470.into()).await.unwrap().unwrap().block_producer;
        let producer_after_2 = client.block_by_height(9704471.into()).await.unwrap().unwrap().block_producer;
        let producer_after_3 = client.block_by_height(9704472.into()).await.unwrap().unwrap().block_producer;
        let producer_after_4 = client.block_by_height(9704473.into()).await.unwrap().unwrap().block_producer;
        let producer_after_5 = client.block_by_height(9704474.into()).await.unwrap().unwrap().block_producer;
        let latest = client.block_by_height(9704674.into()).await.unwrap().unwrap().block_producer;
        // assert_eq!(producer_before, producer_before_1);
        // assert_ne!(producer_before, producer_after);
        // assert_eq!(producer_after_1, producer_after_2);
        // assert_eq!(producer_after_1, producer_after);
        println!("producer_before_4: {:?}", producer_before_4);
        println!("producer_before_3: {:?}", producer_before_3);
        println!("producer_before_2: {:?}", producer_before_2);
        println!("producer_before_1: {:?}", producer_before_1);
        println!("producer_before: {:?}", producer_before);
        println!("producer_after: {:?}", producer_after);
        println!("producer_after_1: {:?}", producer_after_1);
        println!("producer_after_2: {:?}", producer_after_2);
        println!("producer_after_3: {:?}", producer_after_3);
        println!("producer_after_4: {:?}", producer_after_4);
        println!("producer_after_5: {:?}", producer_after_5);
        println!("latest: {:?}", latest);
        // let request = PaginationRequest {
        //     cursor: None,
        //     results: 1,
        //     direction: PageDirection::Backward,
        // };
        // let full_block = client.full_blocks(request).await;
        //
        // assert!(full_block.is_ok(), "{full_block:?}");
    }
}
