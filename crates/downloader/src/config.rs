use clap::Parser;
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
    /// Block to start from.
    #[clap(long, env = "START_BLOCK")]
    pub start_block: u64,
}

impl Config {
    /// Read configuration from environment variables.
    pub fn read_from_env() -> anyhow::Result<Self> {
        Ok(Self {
            ethereum_rpc_url: std::env::var("ETHEREUM_RPC_URL")
                .map_err(|_| anyhow::anyhow!("env var ETHEREUM_RPC_URL not found"))?
                .parse()?,
            beacon_rpc_url: std::env::var("BEACON_RPC_URL")
                .map_err(|_| anyhow::anyhow!("env var BEACON_RPC_URL not found"))?
                .parse()?,
            blob_contract: std::env::var("BLOB_CONTRACT")
                .map_err(|_| anyhow::anyhow!("env var BLOB_CONTRACT not found"))?
                .parse()
                .map_err(|_| anyhow::anyhow!("Unable to decode the address"))?,
            start_block: std::env::var("START_BLOCK")
                .map_err(|_| anyhow::anyhow!("env var START_BLOCK not found"))?
                .parse()?,
        })
    }
}
