use clap::Parser;
use fuel_core::service::genesis::NotifyCancel;
use url::Url;

/// Run the Fuel network watchtower.
#[derive(Debug, Clone, Parser)]
pub struct Command {
    #[clap(flatten)]
    pub fuel_core: fuel_core_bin::cli::run::Command,

    /// The URL of the fuel sentry that supports GraphQL.
    #[arg(long = "url", env)]
    pub fuel_sentry_url: Url,
}

pub async fn exec(command: Command) -> anyhow::Result<()> {
    let (service, shutdown_listener) =
        fuel_core_bin::cli::run::get_service_with_shutdown_listeners(command.fuel_core)
            .await?;

    tokio::select! {
        result = service.start_and_await() => {
            result?;
        }
        _ = shutdown_listener.wait_until_cancelled() => {
            service.send_stop_signal();
        }
    }

    tokio::select! {
        result = service.await_shutdown() => {
            result?;
        }
        _ = shutdown_listener.wait_until_cancelled() => {}
    }

    service.send_stop_signal_and_await_shutdown().await?;

    Ok(())
}
