use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::{AppConfig, LogFormat};

pub fn init(config: &AppConfig) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("teatro=info,tower_http=info"));

    let fmt_layer = match config.log_format {
        LogFormat::Compact => fmt::layer()
            .compact()
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .boxed(),
        LogFormat::Json => fmt::layer()
            .json()
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .boxed(),
        LogFormat::Pretty => fmt::layer()
            .pretty()
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .boxed(),
    };

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init();
}
