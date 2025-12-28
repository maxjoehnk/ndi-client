use color_eyre::eyre::Context;
use tracing::metadata::LevelFilter;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use crate::config::Config;
use crate::ndi::NdiReceiver;
use crate::source_selector::SourceSelector;
use renderer::window::NdiUserEvent;

mod config;
mod ndi;
mod renderer;
mod source_selector;

#[tokio::main(flavor = "current_thread")]
async fn main() -> color_eyre::Result<()> {
    initialize_logger()?;
    let config = Config::read()?;

    let (refreshed_sources_tx, refreshed_sources_rx) = std::sync::mpsc::channel::<()>();
    let source_selector = SourceSelector::new(config);

    let ndi_receiver = NdiReceiver::new(source_selector.clone(), move || {
        let _ = refreshed_sources_tx.send(());
    })?;
    let renderer = renderer::Renderer::new(source_selector).await?;

    let proxy = renderer.create_proxy();
    ndi_receiver.listen(move |monitor_id, frame| {
        proxy.send_event(NdiUserEvent::ReceivedFrame(monitor_id, frame))?;

        Ok(())
    })?;

    let proxy = renderer.create_proxy();
    std::thread::spawn(move || {
        for _ in refreshed_sources_rx.iter() {
            let _ = proxy.send_event(NdiUserEvent::RefreshedSources);
        }
    });

    renderer.launch()?;

    Ok(())
}

fn initialize_logger() -> color_eyre::Result<()> {
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env()?,
        )
        .finish()
        .with(tracing_tracy::TracyLayer::default());
    tracing::subscriber::set_global_default(subscriber).context("tracing setup")?;
    Ok(())
}
