use fuel_core::{
    database::{
        commit_changes_with_height_update,
        database_description::DatabaseDescription,
        Database,
    },
    fuel_core_graphql_api,
    fuel_core_graphql_api::storage::da_compression::DaCompressedBlocks,
};
use fuel_core_storage::{
    iter::{
        IterDirection,
        IteratorOverTable,
    },
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
    type Column = fuel_core_graphql_api::storage::Column;
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

impl<'a> StorageTransactionExtension for CompressionTransaction<'a> {
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
            iter.iter_all::<DaCompressedBlocks>(Some(IterDirection::Reverse))
                .map(|result| result.map(|(height, _)| height))
                .try_collect()
        })
    }
}
