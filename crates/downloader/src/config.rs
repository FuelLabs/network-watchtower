use clap::Parser;
use fuel_core_types::fuel_types::BlockHeight;
use reqwest::Url;

#[derive(Parser, Debug, Clone)]
pub struct Config {
    /// URL to the Ethereum RPC endpoint.
    /// For example `https://sepolia.infura.io/v3/API_KEY_HERE`.
    #[clap(long, env = "ETHEREUM_RPC_URL")]
    pub ethereum_rpc_url: Url,
    /// URL to the Ethereum consensus/beacon layer RPC endpoint.
    /// For example `https://ethereum-sepolia.core.chainstack.com/beacon/API_KEY_HERE`.
    #[clap(long, env = "BEACON_RPC_URL")]
    pub beacon_rpc_url: Url,
    /// Contract to monitor for new blobs.
    /// For example `0xB0B3682211533cB7C1a3Bcb0e0Dd4349fF000d75`.
    #[clap(long, env = "BLOB_CONTRACT")]
    pub blob_contract: fuel_core_types::fuel_types::Bytes20,
    /// DA block to start downloading from.
    #[clap(long, env = "DA_START_BLOCK")]
    pub da_start_block: u64,
    /// Emit fuel blocks starting from this block.
    #[clap(long, env = "NEXT_FUEL_BLOCK")]
    pub next_fuel_block: BlockHeight,
}
