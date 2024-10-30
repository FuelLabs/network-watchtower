use std::collections::BTreeMap;

use fuel_core_compression::VersionedCompressedBlock;
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
