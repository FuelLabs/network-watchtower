use crate::{
    compression_database::{
        Compression,
        CompressionDatabaseExtension,
        CompressionTransaction,
        StorageTransactionExtension,
    },
    config::Config,
};
use anyhow::anyhow;
use fuel_block_fetcher::BlockFetcher;
use fuel_core::{
    database::{
        database_description::{
            on_chain::OnChain,
            relayer::Relayer,
        },
        Database,
    },
    fuel_core_graphql_api::{
        da_compression::{
            DbTx,
            DecompressDbTx,
        },
        ports::DatabaseBlocks,
        storage::da_compression::DaCompressedBlocks,
    },
    service::{
        adapters::{
            BlockImporterAdapter,
            ExecutorAdapter,
        },
        FuelService,
    },
    state::historical_rocksdb::StateRewindPolicy,
};
use fuel_core_compression::{
    decompress::decompress,
    Config as CompressionConfig,
    VersionedCompressedBlock,
};
use fuel_core_poa::ports::BlockImporter;
use fuel_core_storage::{
    not_found,
    transactional::{
        AtomicView,
        HistoricalView,
        WriteTransaction,
    },
    StorageAsMut,
};
use fuel_core_types::{
    blockchain::{
        block::PartialFuelBlock,
        consensus::{
            poa::PoAConsensus,
            Consensus,
        },
        primitives::DaBlockHeight,
        SealedBlock,
    },
    fuel_tx::field::{
        InputContract,
        MintGasPrice,
    },
    fuel_types::BlockHeight,
    services::{
        block_importer::ImportResult,
        block_producer::Components,
        Uncommitted,
    },
};
use futures::StreamExt;
use std::sync::Arc;

pub struct BlockSyncer {
    compression_config: CompressionConfig,
    block_fetcher: BlockFetcher,
    on_chain_database: Database<OnChain>,
    database: Database<Compression>,
    relayer: fuel_core_relayer::SharedState<Database<Relayer>>,
    executor: ExecutorAdapter,
    block_importer: BlockImporterAdapter,
}

impl BlockSyncer {
    pub fn new(fuel_service: &FuelService, config: Config) -> anyhow::Result<Self> {
        let block_fetcher = BlockFetcher::new(config.fuel_sentry_url)?;
        let fuel_config = &fuel_service.shared.config;
        let db_path = fuel_config.combined_db_config.database_path.clone();
        let compression_config = CompressionConfig {
            temporal_registry_retention: config.compression.into(),
        };

        let database = Database::open_rocksdb(
            db_path.as_path(),
            None,
            StateRewindPolicy::RewindFullRange,
        )?;
        let on_chain_database = fuel_service.shared.database.on_chain().clone();
        let block_importer = fuel_service.shared.block_importer.clone();
        let executor = fuel_service.shared.executor.clone();
        let relayer = fuel_service
            .shared
            .relayer
            .clone()
            .ok_or(anyhow!("Relayer is not enabled"))?;

        Ok(Self {
            compression_config,
            block_fetcher,
            on_chain_database,
            database,
            relayer,
            executor,
            block_importer,
        })
    }

    pub fn on_chain_latest_height(&self) -> anyhow::Result<BlockHeight> {
        let height = self
            .on_chain_database
            .latest_height()
            .ok_or(not_found!("No genesis block"))?;
        Ok(height)
    }

    pub fn compression_latest_height(&self) -> Option<BlockHeight> {
        self.database.latest_height()
    }

    pub fn compression_latest_da_height(&self) -> anyhow::Result<DaBlockHeight> {
        let block_height = self
            .compression_latest_height()
            .ok_or(not_found!("No compression data found in the database"))?;

        let block = self.on_chain_database.latest_view()?.block(&block_height)?;

        Ok(block.header().da_height)
    }

    pub async fn sync_as_much_as_possible_from_graphql(&mut self) -> anyhow::Result<()> {
        tracing::info!("Syncing blocks from GraphQL");
        let current_height: u32 = self.on_chain_latest_height()?.into();
        let latest_remote_height: u32 = self.block_fetcher.last_height().await?.into();

        let block_fetcher = Arc::new(self.block_fetcher.clone());
        let next_height = current_height.saturating_add(1);

        let mut stream = futures::stream::iter(next_height..latest_remote_height)
            .chunks(40)
            .map(|chunk| {
                let start = chunk[0];
                let last = start
                    .saturating_add(u32::try_from(chunk.len()).expect("It is 40 above"));

                start..last
            })
            .map(move |range| {
                let block_fetcher = block_fetcher.clone();
                async move { block_fetcher.blocks_for(range).await }
            })
            .buffered(8);

        let (sender, mut receiver) = tokio::sync::mpsc::channel(1024);

        tokio::task::spawn(async move {
            while let Some(Ok(blocks)) = stream.next().await {
                for block in blocks {
                    let result = sender.send(block).await;

                    if result.is_err() {
                        break;
                    }
                }
            }
        });

        while let Some(block) = receiver.recv().await {
            let block = block.block;
            self.relayer
                .await_at_least_synced(&block.entity.header().da_height)
                .await?;
            self.block_importer.execute_and_commit(block).await?;
        }

        Ok(())
    }

    pub async fn sync_compression_from_graphql(&mut self) -> anyhow::Result<()> {
        tracing::info!("Syncing compressed blocks from GraphQL");
        let target_compression_height: u32 = self.on_chain_latest_height()?.into();

        // We assume that block are produces each second.
        // TODO: We can use binary search to find the first block from which we
        //  need to get compression data.
        let compression_window = u32::try_from(
            self.compression_config
                .temporal_registry_retention
                .as_secs(),
        )?;

        let start_compression_height =
            target_compression_height.saturating_sub(compression_window);

        let mut latest_compression_height = self.compression_latest_height();
        let start_compression_height = start_compression_height.max(
            u32::from(latest_compression_height.unwrap_or_default()).saturating_add(1),
        );

        let block_fetcher = Arc::new(self.block_fetcher.clone());

        let mut stream =
            futures::stream::iter(start_compression_height..target_compression_height)
                .chunks(40)
                .map(|chunk| {
                    let start = chunk[0];
                    let last = start.saturating_add(
                        u32::try_from(chunk.len()).expect("It is 40 above"),
                    );

                    start..last
                })
                .map(move |range| {
                    let block_fetcher = block_fetcher.clone();
                    async move { block_fetcher.compressed_blocks_for(range).await }
                })
                .buffered(8);

        let (sender, mut receiver) = tokio::sync::mpsc::channel(1024);

        tokio::task::spawn(async move {
            while let Some(Ok(blocks)) = stream.next().await {
                for block in blocks {
                    let result = sender.send(block).await;

                    if result.is_err() {
                        break;
                    }
                }
            }
        });

        while let Some(block) = receiver.recv().await {
            match block {
                None => {
                    // If we receive `None` while fetching compressed blocks and before we
                    // had a compressed data, then break since the compression provider is
                    // malicious or has bad state.
                    if latest_compression_height.is_some() {
                        break
                    } else {
                        tracing::info!("Skipping block without compression data");
                        // If we didn't have any compression state before,
                        // then skip this block until we find the first non-empty block.
                        continue
                    }
                }
                Some(block) => {
                    self.commit_compression(&block).await?;
                    latest_compression_height = Some(*block.height());
                }
            }
        }

        Ok(())
    }

    async fn commit_compression(
        &mut self,
        block: &VersionedCompressedBlock,
    ) -> anyhow::Result<()> {
        tracing::info!("Apply compression for block: {:?}", block.height());
        let tx = self.apply_compression(block).await?;
        tx.commit_compression()?;

        Ok(())
    }

    pub async fn import_block_from_da(
        &mut self,
        compressed_block: VersionedCompressedBlock,
    ) -> anyhow::Result<()> {
        let block_height = *compressed_block.height();
        let on_chain_height = self.on_chain_latest_height()?;
        let less_than_on_chain = block_height <= on_chain_height;

        let compression_block_height = self.compression_latest_height();

        if less_than_on_chain {
            if let Some(compression_block_height) = compression_block_height {
                if *compressed_block.height() <= compression_block_height {
                    tracing::info!(
                        "Skipping block {} from DA since it is already known",
                        compressed_block.height()
                    );
                    return Ok(())
                }
            }
        }

        tracing::info!(
            "Importing compressed block {} from DA",
            compressed_block.height()
        );

        let compression_changes = self
            .apply_compression(&compressed_block)
            .await?
            .into_changes();

        if !less_than_on_chain {
            let decompressed_block = self.decompress_block(compressed_block).await?;

            let mut transactions = decompressed_block.transactions;
            let maybe_mint_tx = transactions.pop();
            let mint_tx =
                maybe_mint_tx
                    .and_then(|tx| tx.as_mint().cloned())
                    .ok_or(anyhow!(
                        "The last transaction in the block should be a mint transaction"
                    ))?;

            let gas_price = *mint_tx.gas_price();
            let coinbase_recipient = mint_tx.input_contract().contract_id;

            let component = Components {
                header_to_produce: decompressed_block.header,
                transactions_source: transactions,
                coinbase_recipient,
                gas_price,
            };

            let (execution_result, changes) = self
                .executor
                .produce_without_commit_from_vector(component)?
                .into();

            let sealed_block = SealedBlock {
                entity: execution_result.block,
                consensus: Consensus::PoA(PoAConsensus {
                    // If data is posted to DA it already a proof that it was signed by
                    // the block producer.
                    // We only need to verify that execution was done correctly.
                    signature: Default::default(),
                }),
            };

            let import_result = Uncommitted::new(
                ImportResult::new_from_local(
                    sealed_block,
                    execution_result.tx_status,
                    execution_result.events,
                ),
                changes,
            );

            self.block_importer.commit_result(import_result).await?;
        }

        let compression_block_height = compression_block_height.unwrap_or_default();
        if block_height > compression_block_height {
            self.database.commit_compression(compression_changes)?;
        }

        Ok(())
    }

    async fn apply_compression(
        &mut self,
        block: &VersionedCompressedBlock,
    ) -> anyhow::Result<CompressionTransaction> {
        let mut tx = self.database.write_transaction();

        let mut compression_tx = tx.write_transaction();

        compression_tx
            .storage_as_mut::<DaCompressedBlocks>()
            .insert(block.height(), block)?;

        let mut compression_db = DbTx {
            db_tx: &mut compression_tx,
        };

        let VersionedCompressedBlock::V0(block) = block;

        block
            .registrations
            .write_to_registry(&mut compression_db, block.header.consensus.time)?;
        compression_tx.commit()?;

        Ok(tx)
    }

    async fn decompress_block(
        &mut self,
        block: VersionedCompressedBlock,
    ) -> anyhow::Result<PartialFuelBlock> {
        let on_chain_database = self.on_chain_database.clone();
        let mut tx = self.database.write_transaction();

        let decompression_db = DecompressDbTx {
            db_tx: DbTx {
                db_tx: &mut tx.write_transaction(),
            },
            onchain_db: on_chain_database,
        };

        let decompressed_block =
            decompress(self.compression_config, decompression_db, block).await?;

        Ok(decompressed_block)
    }
}
