//! Listen for usb device events

//use futures::StreamExt;
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
    info!("Listening to Serial Port Detect events for 5 seconds");

    // Create a timeout to end our demo
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(5));

    // Listen to serialport events
    let mut stream = serialport_detect::listen().unwrap();

    // Merge the streams
    loop {
        tokio::select! {
            result = stream.next() => {
                match result {
                    Some(Ok(event)) => {
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
                    Some(Err(error)) => error!(?error, "device event error"),
                    None => {
                        info!("demo over");
                        stream.cancel();
                        break;
                    }
                }
            },
            _ = tokio::time::sleep_until(timeout.deadline()) => {
                info!("demo over");
                stream.cancel();
                break
            }
        }
    }

    // End
    Ok(())
}
