use fuel_network_watchtower_downloader::{Downloader, Config};
use futures_util::StreamExt;

#[tokio::main]
async fn main() {
    let config = Config::read_from_env().unwrap();
    let mut downloader = Downloader::new(config).unwrap().stream();

    while let Some(result) = downloader.next().await {
        match result {
            Ok((header, bundle)) => {
                println!("Header: {:?}", header);
                println!("Bundle: {:?}", bundle);
            },
            Err(e) => {
                eprintln!("Error: {:?}", e);
                break;
            },
        }
    }
}