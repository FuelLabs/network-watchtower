use fuel_core::database::{
    commit_changes_with_height_update,
    database_description::DatabaseDescription,
    Database,
};
use fuel_core_compression_service::storage::{
    column::CompressionColumn,
    CompressedBlocks,
};
use fuel_core_storage::{
    iter::{
        IterDirection,
        IteratorOverTable,
    },
    merkle::column::MerkleizedColumn,
    transactional::{
        Changes,
        StorageTransaction,
    },
    Result as StorageResult,
};
use fuel_core_types::fuel_types::BlockHeight;
use itertools::Itertools;

#[derive(Copy, Clone, Debug)]
pub struct Compression;

impl DatabaseDescription for Compression {
    type Column = MerkleizedColumn<CompressionColumn>;
    type Height = BlockHeight;

    fn version() -> u32 {
        0
    }

    fn name() -> String {
        "compression".to_string()
    }

    fn metadata_column() -> Self::Column {
        Self::Column::Metadata
    }

    fn prefix(_: &Self::Column) -> Option<usize> {
        None
    }
}

pub type CompressionTransaction<'a> = StorageTransaction<&'a mut Database<Compression>>;

pub trait StorageTransactionExtension {
    fn commit_compression(self) -> StorageResult<()>;
}

impl StorageTransactionExtension for CompressionTransaction<'_> {
    fn commit_compression(self) -> StorageResult<()> {
        let (db, changes) = self.into_inner();

        db.commit_compression(changes)
    }
}

pub trait CompressionDatabaseExtension {
    fn commit_compression(&mut self, changes: Changes) -> StorageResult<()>;
}

impl CompressionDatabaseExtension for Database<Compression> {
    fn commit_compression(&mut self, changes: Changes) -> StorageResult<()> {
        commit_changes_with_height_update(self, changes, |iter| {
            iter.iter_all::<CompressedBlocks>(Some(IterDirection::Reverse))
                .map(|result| result.map(|(height, _)| height))
                .try_collect()
        })
    }
}
