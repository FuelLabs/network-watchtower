use fuel_network_watchtower_downloader::{
    Config,
    Downloader,
};
use futures_util::StreamExt;

#[tokio::main]
async fn main() {
    let config = Config::read_from_env().unwrap();
    let mut downloader = Downloader::new(config).stream();

    while let Some(result) = downloader.next().await {
        match result {
            Ok(block) => {
                println!("Block: {:?}", block);
            }
            Err(e) => {
                eprintln!("Error: {:?}", e);
                break;
            }
        }
    }
}
