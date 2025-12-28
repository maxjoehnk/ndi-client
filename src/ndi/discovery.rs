use std::sync::mpsc::Sender;
use ndi::Source;

#[derive(Debug, Clone)]
pub struct NdiSource {
    pub name: String,
    pub source: Source,
}

pub fn discover_sources(sender: Sender<Vec<NdiSource>>, on_discovered: impl Fn()) -> color_eyre::Result<()> {
    tracing::debug!("Setting up NDI finder");
    let find = ndi::find::FindBuilder::new().build()?;

    loop {
        tracing::debug!("Finding NDI sources");
        let sources = find.current_sources(u128::MAX)?;

        tracing::info!("Found NDI sources: {sources:?}");

        let sources = sources
            .into_iter()
            .map(|source| NdiSource {
                name: source.get_name(),
                source,
            })
            .collect::<Vec<_>>();

        sender.send(sources)?;
        on_discovered();

        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}
