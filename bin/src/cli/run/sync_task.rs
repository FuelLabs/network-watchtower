use fuel_block_syncer::{
    block_syncer::BlockSyncer,
    config::Config as BlockSyncConfig,
};
use fuel_core::service::FuelService;
use fuel_core_compression::VersionedCompressedBlock;
use fuel_core_services::{
    stream::{
        BoxStream,
        IntoBoxStream,
    },
    EmptyShared,
    RunnableService,
    RunnableTask,
    ServiceRunner,
    StateWatcher,
    TaskNextAction,
};
use fuel_network_watchtower_downloader::{
    Config as DownloaderConfig,
    Downloader,
};
use futures_util::StreamExt;

pub struct UninitializedTask {
    block_syncer: BlockSyncer,
    downloader_config: DownloaderConfig,
}

pub struct Task {
    block_syncer: BlockSyncer,
    stream_from_da: BoxStream<anyhow::Result<VersionedCompressedBlock>>,
}

#[async_trait::async_trait]
impl RunnableService for UninitializedTask {
    const NAME: &'static str = "CompressionSyncer";
    type SharedData = EmptyShared;
    type Task = Task;
    type TaskParams = ();

    fn shared_data(&self) -> Self::SharedData {
        EmptyShared
    }

    async fn into_task(
        mut self,
        state_watcher: &StateWatcher,
        _: Self::TaskParams,
    ) -> anyhow::Result<Self::Task> {
        let mut state_watcher = state_watcher.clone();
        tokio::select! {
            _ = state_watcher.wait_stopping_or_stopped() => {
                return Err(anyhow::anyhow!("Received stop signal during \
                    synchronization via the GraphQl"))
            }
            result = self.block_syncer.sync_as_much_as_possible_from_graphql() => {
                result?;
            }
        }

        tokio::select! {
            _ = state_watcher.wait_stopping_or_stopped() => {
                return Err(anyhow::anyhow!("Received stop signal during synchronization \
                    of compression data via the GraphQl"))
            }
            result = self.block_syncer.sync_compression_from_graphql() => {
                result?;
            }
        }

        let mut config = self.downloader_config;
        config
            .set_da_start_block(self.block_syncer.compression_latest_da_height()?.into());
        config.set_next_fuel_block(self
            .block_syncer
            .compression_latest_height()
            .expect(
                "If we have the latest da height, we have the latest fuel height as well",
            )
            .succ()
            .expect("Out of Fuel block heights"));
        let downloader_stream = Downloader::new(config).stream().into_boxed();

        Ok(Task {
            block_syncer: self.block_syncer,
            stream_from_da: downloader_stream,
        })
    }
}

impl RunnableTask for Task {
    async fn run(&mut self, watcher: &mut StateWatcher) -> TaskNextAction {
        tokio::select! {
            _ = watcher.while_started() => {
                TaskNextAction::Stop
            }
            compressed_block = self.stream_from_da.next() => {
                if let Some(compressed_block) = compressed_block {
                    let compressed_block = match compressed_block {
                        Ok(compressed_block) => {
                            compressed_block
                        }
                        Err(err) => {
                            tracing::error!("Error while downloading compressed block: {:?}", err);
                            return TaskNextAction::Stop;
                        }
                    };

                    let result = self.block_syncer.import_block_from_da(compressed_block).await;

                    if let Err(err) = result {
                        tracing::error!("Error while importing compressed block: {:?}", err);
                        TaskNextAction::Stop
                    } else {
                        TaskNextAction::Continue
                    }
                } else {
                    TaskNextAction::Stop
                }
            }
        }
    }

    async fn shutdown(self) -> anyhow::Result<()> {
        // Do nothing for now
        Ok(())
    }
}

pub fn new_service(
    fuel_service: &FuelService,
    config: BlockSyncConfig,
    downloader_config: DownloaderConfig,
) -> anyhow::Result<ServiceRunner<UninitializedTask>> {
    let task = UninitializedTask {
        block_syncer: BlockSyncer::new(fuel_service, config)?,
        downloader_config,
    };

    Ok(ServiceRunner::new(task))
}
