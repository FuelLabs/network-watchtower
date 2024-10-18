use crate::cli::run::sync_task::{
    new_service,
    UninitializedTask,
};
use fuel_block_syncer::config::Config as BlockSyncerConfig;
use fuel_core::service::FuelService;
use fuel_core_services::{
    Service,
    ServiceRunner,
};
use fuel_network_watchtower_downloader::Config as DownloaderConfig;

pub struct SyncService {
    fuel_core: FuelService,
    sync: ServiceRunner<UninitializedTask>,
}

impl SyncService {
    pub fn new(
        fuel_core: FuelService,
        config: BlockSyncerConfig,
        downloader_config: DownloaderConfig,
    ) -> anyhow::Result<Self> {
        let sync = new_service(&fuel_core, config, downloader_config)?;
        Ok(Self { fuel_core, sync })
    }

    pub async fn start_and_await(&self) -> anyhow::Result<()> {
        self.fuel_core.start_and_await().await?;
        self.sync.start_and_await().await?;

        Ok(())
    }

    pub fn send_stop_signal(&self) {
        self.fuel_core.send_stop_signal();
        self.sync.stop();
    }

    pub async fn await_shutdown(&self) -> anyhow::Result<()> {
        tokio::select! {
            result = self.sync.await_stop() => {
                result?;
            }
            result = self.fuel_core.await_shutdown() => {
                result?;
            }
        }

        Ok(())
    }

    pub async fn send_stop_signal_and_await_shutdown(&self) -> anyhow::Result<()> {
        self.sync.stop_and_await().await?;
        self.fuel_core.send_stop_signal_and_await_shutdown().await?;

        Ok(())
    }
}
