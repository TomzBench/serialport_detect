//! Listen for usb device events

use tokio_stream::StreamExt as TokioStreamExt;
use tracing::{error, info};
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
        .with(LevelFilter::TRACE)
        .init();

    // Welcome message
    info!("Listening to Serial Port Detect events for 15 seconds");

    // Create a timeout to end our demo
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(15));

    // Listen to serialport events
    let (abort, mut stream) = serialport_detect::listen().unwrap();

    // Merge the streams
    loop {
        tokio::select! {
            result = stream.next() => {
                match result {
                    Some(Ok(event)) => {
                        info!(
                            action = ?event.event,
                            port = ?event.device.port,
                            vid = ?event.device.vid,
                            pid = ?event.device.pid,
                            serial = ?event.device.serial,
                            manufacture = ?event.device.manufacturer,
                            product = ?event.device.product,
                            "device event"
                        );
                    }
                    Some(Err(error)) => error!(?error, "device event error"),
                    None => {
                        info!("demo over");
                        drop(abort);
                        break;
                    }
                }
            },
            _ = tokio::time::sleep_until(timeout.deadline()) => {
                info!("demo over");
                drop(abort);
                break
            }
        }
    }

    // End
    Ok(())
}
