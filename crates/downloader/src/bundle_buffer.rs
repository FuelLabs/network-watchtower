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
