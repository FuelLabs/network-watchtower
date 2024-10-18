use clap::Parser;

/// Block syncer configuration.
#[derive(Debug, Clone, Parser)]
pub struct Config {
    /// The URL of the fuel sentry that supports GraphQL.
    #[arg(long = "url", env)]
    pub fuel_sentry_url: url::Url,

    /// Produce and store DA compressed blocks with the given retention time.
    #[arg(long = "compression", env)]
    pub compression: humantime::Duration,
}
