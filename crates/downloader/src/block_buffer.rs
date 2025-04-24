use std::collections::BTreeMap;

use fuel_core_compression::{
    VersionedBlockPayload,
    VersionedCompressedBlock,
};
use fuel_core_types::fuel_types::BlockHeight;

#[derive(Default)]
pub struct BlockBuffer {
    blocks: BTreeMap<BlockHeight, VersionedCompressedBlock>,
}

impl BlockBuffer {
    pub fn push(&mut self, block: VersionedCompressedBlock) {
        self.blocks.insert(*block.height(), block);
    }

    /// Given "high water mark" block height, remove all blocks below it.
    pub fn prune_below(&mut self, height: BlockHeight) {
        self.blocks = self.blocks.split_off(&height);
    }

    /// If the first entry has given height, remove and return it.
    pub fn pop_height(
        &mut self,
        height: BlockHeight,
    ) -> Option<VersionedCompressedBlock> {
        if let Some(head) = self.blocks.first_entry() {
            if head.key() == &height {
                Some(head.remove())
            } else {
                None
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use fuel_core_compression::VersionedCompressedBlock;

    fn mock_block(height: u32) -> VersionedCompressedBlock {
        let mut block = VersionedCompressedBlock::default();
        // we probably should expose test helpers in the compression crate
        match block {
            VersionedCompressedBlock::V0(ref mut v0) => {
                v0.header.consensus.height = height.into();
            }
            _ => panic!("unexpected block version")
        }
        block
    }

    #[test]
    fn block_buffer_happy_path() {
        use super::*;

        let mut buffer = BlockBuffer::default();

        buffer.push(mock_block(1));
        buffer.push(mock_block(3));
        buffer.push(mock_block(2));

        buffer.prune_below(2u32.into());

        let block = buffer.pop_height(2u32.into());
        assert_eq!(block, Some(mock_block(2)));

        let block = buffer.pop_height(3u32.into());
        assert_eq!(block, Some(mock_block(3)));
    }
}
