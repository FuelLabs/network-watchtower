use fuel_network_watchtower_downloader::{Downloader, Config};

#[tokio::main]
async fn main() {
    let config = Config::read_from_env().unwrap();
    let mut downloader = Downloader::new(config).unwrap();

    let blobs = downloader.next_block_bundles().await.unwrap();

    for (header, bundle) in blobs {
        println!("Header: {:?}", header);
        println!("Bundle: {:?}", bundle);
    }
}