use std::collections::HashMap;

use fuel_block_committer_encoding::{
    blob::{
        self,
        Blob,
        Header,
        HeaderV1,
    },
    bundle::{
        self,
        Bundle,
        BundleV1,
    },
};

/// Holds a buffer for each incomplete bundle. Bundles are removed from the buffer when they are complete.
#[derive(Default)]
pub struct IncompleteBundleBuffers {
    /// Mapping from bundle id to a sorted list of blobs
    bundle_buffer_by_id: HashMap<u32, Vec<(HeaderV1, Blob)>>,
}
impl IncompleteBundleBuffers {
    /// Insert into the buffer of this bundle, keeping the blobs inside it sorted.
    /// Returns an error on duplicate insertion, or if the bundle cannot be decoded.
    pub fn insert(&mut self, blob: Blob) -> anyhow::Result<Inserted> {
        let header = blob::Decoder::default().read_header(&blob)?;
        let Header::V1(header) = header;
        let bundle_id = header.bundle_id;

        let blobs = self.bundle_buffer_by_id.entry(bundle_id).or_default();
        if let Err(idx) =
            blobs.binary_search_by_key(&header.idx, |(header, _)| header.idx)
        {
            tracing::info!(
                "Inserted blob for bundle {}, at index {}, last {}",
                header.bundle_id,
                header.idx,
                header.is_last,
            );
            blobs.insert(idx, (header, blob));
        } else {
            anyhow::bail!("Duplicate blob with index {}", header.idx);
        }

        // Check if the bundle is complete
        if let Some((last_header, _)) = blobs.last() {
            if !last_header.is_last {
                return Ok(Inserted::Incomplete);
            }
        }
        for (expected_idx, (header, _)) in blobs.iter().enumerate() {
            if header.idx as usize != expected_idx {
                return Ok(Inserted::Incomplete);
            }
        }

        // Now we do have a complete bundle
        let blobs = self
            .bundle_buffer_by_id
            .remove(&bundle_id)
            .expect("This was inserted above");
        let blobs: Vec<Blob> = blobs.into_iter().map(|(_, blob)| blob).collect();

        let bundle_bytes = blob::Decoder::default().decode(&blobs)?;
        let bundle: Bundle = bundle::Decoder::default().decode(&bundle_bytes)?;
        let Bundle::V1(bundle) = bundle;
        Ok(Inserted::Complete(bundle))
    }
}

pub enum Inserted {
    Complete(BundleV1),
    Incomplete,
}

#[cfg(test)]
mod tests {
    use fuel_block_committer_encoding::{
        blob,
        bundle::{
            self,
            Bundle,
            BundleV1,
        },
    };
    use proptest::prelude::*;
    use rand::{
        seq::SliceRandom,
        SeedableRng,
    };

    use super::{
        IncompleteBundleBuffers,
        Inserted,
    };

    proptest! {
        #[test]
        fn single_bundle_any_order_works(
            blocks in prop::collection::vec(prop::collection::vec(0..=u8::MAX, 1..12_346), 0..7),
            id in 0..=u32::MAX,
            shuffle_seed in 0..=u64::MAX,
        ) {
            let bundle = Bundle::V1(BundleV1 {
                blocks: blocks.clone(),
            });
            let data = bundle::Encoder::default().encode(bundle).unwrap();
            let mut encoded_parts = blob::Encoder::default().encode(&data, id).unwrap();

            let mut rng = rand::rngs::StdRng::seed_from_u64(shuffle_seed);
            encoded_parts.shuffle(&mut rng);

            let mut buffers = IncompleteBundleBuffers::default();

            let final_part = encoded_parts.pop().unwrap();
            for part in encoded_parts {
                let result = buffers.insert(part).unwrap();
                assert!(matches!(result, Inserted::Incomplete));
            }

            let result = buffers.insert(final_part).unwrap();
            let Inserted::Complete(bundle) = result else {
                panic!("Expected a complete bundle");
            };

            assert_eq!(bundle.blocks, blocks);
        }

        #[test]
        fn multiple_bundles_interspersed_works(
            seed in 0..=u64::MAX,
            mut bundle_blocks in prop::collection::vec((0..=u32::MAX, prop::collection::vec(prop::collection::vec(0..=u8::MAX, 1..1245), 0..6)), 0..7),
        ) {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

            let mut parts_for_each_bundle = Vec::new();
            for (id, blocks) in &bundle_blocks {
                let bundle = Bundle::V1(BundleV1 {
                    blocks: blocks.clone(),
                });
                let data = bundle::Encoder::default().encode(bundle).unwrap();
                let mut encoded_parts = blob::Encoder::default().encode(&data, *id).unwrap();
                encoded_parts.shuffle(&mut rng);
                parts_for_each_bundle.push(encoded_parts);
            }


            let mut buffers = IncompleteBundleBuffers::default();
            while !parts_for_each_bundle.is_empty() {
                let i = rng.gen_range(0..parts_for_each_bundle.len());
                let mut encoded_parts = parts_for_each_bundle.remove(i);

                let part = encoded_parts.pop().unwrap();
                let result = buffers.insert(part).unwrap();

                if encoded_parts.is_empty() {
                    // This was the last part
                    let Inserted::Complete(bundle) = result else {
                        panic!("Expected a complete bundle after the final part");
                    };
                    let i = bundle_blocks.iter().position(|(_, data)| *data == bundle.blocks).expect("Reconstructed bundle not found");
                    bundle_blocks.swap_remove(i);
                } else {
                    // There's more to this bundle, re-insert the rest
                    assert!(matches!(result, Inserted::Incomplete));
                    parts_for_each_bundle.push(encoded_parts);
                }
            }
        }
    }
}
