pub use self::discovery::NdiSource;
use crate::source_selector::{MonitorId, SourceSelector};
use color_eyre::eyre::Context;
use image::DynamicImage;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

mod discovery;
mod receiver;

pub struct NdiReceiver {
    source_selector: SourceSelector,
    receiver: Receiver<Vec<NdiSource>>,
}

impl NdiReceiver {
    pub fn new<TCallback: 'static + Fn() + Send + Sync>(
        source_selector: SourceSelector,
        on_discovered: TCallback,
    ) -> color_eyre::Result<Self> {
        tracing::debug!("Initializing NDI");
        ndi::initialize().context("initializing ndi")?;

        let (sender, receiver) = std::sync::mpsc::channel::<Vec<NdiSource>>();

        let receiver = Self {
            source_selector,
            receiver,
        };

        receiver.start_discovery(sender, on_discovered)?;

        Ok(receiver)
    }

    pub fn listen<
        TCallback: 'static + Fn(MonitorId, DynamicImage) -> color_eyre::Result<()> + Clone + Send + Sync,
    >(
        self,
        send_frame: TCallback,
    ) -> color_eyre::Result<()> {
        std::thread::Builder::new()
            .name("NDI Receiver".to_string())
            .spawn(move || {
                let mut receivers = HashMap::new();

                loop {
                    if let Ok(sources) = self.receiver.try_recv() {
                        self.source_selector.update_available_sources(sources);
                    }

                    for (monitor_id, source) in self.source_selector.targets()
                        .into_iter()
                        .filter(|(monitor_id, _)| monitor_id.has_window_handle()) {
                        if receivers.contains_key(&monitor_id) {
                            continue;
                        }
                        let id = monitor_id.clone();
                        let send_frame = send_frame.clone();
                        match self.spawn_receiver(
                            monitor_id.as_ref(),
                            move |image| {
                                tracing::trace!("Sending frame to window {id:?}");
                                if let Err(err) = send_frame(id.clone(), image) {
                                    tracing::error!(error = ?err, "Error sending frame to window {id:?}");
                                }
                            },
                            source,
                        ) {
                            Ok(receiver_handle) => receivers.insert(monitor_id, receiver_handle),
                            Err(err) => {
                                tracing::error!(error = ?err, "Failed to spawn receiver for {monitor_id:?}");
                                continue;
                            }
                        };
                    }

                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            })?;

        Ok(())
    }

    fn start_discovery<TCallback: 'static + Fn() + Send + Sync>(
        &self,
        sender: Sender<Vec<NdiSource>>,
        on_discovered: TCallback,
    ) -> color_eyre::Result<()> {
        std::thread::Builder::new()
            .name("NDI Source Discovery".to_string())
            .spawn(|| {
                if let Err(err) = discovery::discover_sources(sender, on_discovered) {
                    tracing::error!(error = ?err, "NDI Source discovery crashed");
                }
            })?;

        Ok(())
    }

    fn spawn_receiver<TCallback: 'static + Fn(DynamicImage) + Send + Sync>(
        &self,
        monitor: &str,
        send_frame: TCallback,
        initial_source: ndi::Source,
    ) -> color_eyre::Result<NdiReceiverHandle> {
        let (tx, rx) = std::sync::mpsc::channel();

        tx.send(initial_source)?;

        std::thread::Builder::new()
            .name(format!("NDI Receiver {monitor}"))
            .spawn(move || {
                if let Err(err) = receiver::recv_ndi(send_frame, rx) {
                    tracing::error!(error = ?err, "NDI Receiver crashed");
                }
            })?;

        Ok(NdiReceiverHandle(tx))
    }
}

struct NdiReceiverHandle(Sender<ndi::Source>);

impl NdiReceiverHandle {
    fn send_source(&self, source: ndi::Source) -> color_eyre::Result<()> {
        self.0.send(source)?;
        Ok(())
    }
}
