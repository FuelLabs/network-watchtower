use crate::cli::run::sync_service::SyncService;
use clap::Parser;
use fuel_block_syncer::config::Config as BlockSyncerConfig;
use fuel_core::{
    service::genesis::NotifyCancel,
    types::fuel_types::BlockHeight,
};
use fuel_network_watchtower_downloader::Config as DownloaderConfig;

mod sync_service;
mod sync_task;

/// Run the Fuel network watchtower.
#[derive(Debug, Clone, Parser)]
pub struct WatchtowerCommand {
    #[clap(flatten)]
    pub fuel_core: fuel_core_bin::cli::run::Command,

    #[clap(flatten)]
    pub block_sync_config: BlockSyncerConfig,

    /// URL to the Ethereum consensus/beacon layer RPC endpoint.
    /// For example `https://ethereum-sepolia.core.chainstack.com/beacon/API_KEY_HERE`.
    #[clap(long, env = "BEACON_RPC_URL")]
    pub beacon_rpc_url: url::Url,

    /// Contract to monitor for new blobs.
    /// For example `0xB0B3682211533cB7C1a3Bcb0e0Dd4349fF000d75`.
    #[clap(long, env = "BLOB_CONTRACT")]
    pub blob_contract: fuel_core::types::fuel_types::Bytes20,
}

pub async fn exec(command: WatchtowerCommand) -> anyhow::Result<()> {
    let eth_rpcs = command
        .fuel_core
        .relayer_args
        .relayer
        .as_ref()
        .ok_or(anyhow::anyhow!(
            "Relayer URL is required for the downloader"
        ))?
        .clone();

    let (service, shutdown_listener) =
        fuel_core_bin::cli::run::get_service_with_shutdown_listeners(command.fuel_core)
            .await?;

    let downloader_config = DownloaderConfig {
        ethereum_rpc_url: eth_rpcs[0].clone(),
        beacon_rpc_url: command.beacon_rpc_url,
        blob_contract: command.blob_contract,
        // We will set these later in the syncer.
        da_start_block: 0,
        next_fuel_block: BlockHeight::new(0),
    };

    let syncer = SyncService::new(service, command.block_sync_config, downloader_config)?;

    tokio::select! {
        result = syncer.start_and_await() => {
            result?;
        }
        _ = shutdown_listener.wait_until_cancelled() => {
            syncer.send_stop_signal();
        }
    }

    tokio::select! {
        result = syncer.await_shutdown() => {
            result?;
        }
        _ = shutdown_listener.wait_until_cancelled() => {}
    }

    syncer.send_stop_signal_and_await_shutdown().await?;

    Ok(())
}
