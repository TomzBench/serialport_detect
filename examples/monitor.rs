//! Listen for usb device events
use futures::StreamExt;
use tracing::info;
use tracing_subscriber::{filter::LevelFilter, fmt, layer::SubscriberExt, prelude::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup logging
    let stdout = fmt::layer()
        .compact()
        .with_ansi(true)
        .with_level(true)
        .with_file(false)
        .with_line_number(false)
        .with_target(true);
    tracing_subscriber::registry()
        .with(stdout)
        .with(LevelFilter::INFO)
        .init();

    // Welcome message
    info!("Application service starting...");

    // Listen to serialport events
    let mut stream = serialport_detect::listen().unwrap();
    while let Some(result) = stream.next().await {
        let event = result?;
        info!(
            action = ?event.event,
            port = ?event.port,
            vid = ?event.meta.vid,
            pid = ?event.meta.pid,
            serial = ?event.meta.serial,
            manufacture = ?event.meta.manufacturer,
            product = ?event.meta.product,
            "device event"
        );
    }

    // End
    info!("demo over");
    Ok(())
}
