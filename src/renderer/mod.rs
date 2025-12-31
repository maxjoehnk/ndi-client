use winit::event_loop::{ControlFlow, EventLoop};
use crate::config::Config;
use crate::renderer::window::NdiClientApp;
use crate::source_selector::SourceSelector;
use self::window::NdiUserEvent;

mod image_renderer;
pub mod window;

pub struct Renderer {
    event_loop: EventLoop<NdiUserEvent>,
    app: NdiClientApp,
    #[allow(dead_code)]
    keep_awake: Option<keep_active::KeepActive>,
}

impl Renderer {
    pub async fn new(source_selector: SourceSelector, config: Option<&Config>) -> color_eyre::Result<Self> {
        let synchronize_screens = config.map(|config| config.synchronize_screens).unwrap_or_default();

        if synchronize_screens {
            tracing::info!("Synchronizing screens by blocking rendering until all screens have received a new frame");
        }

        let event_loop = winit::event_loop::EventLoop::<NdiUserEvent>::with_user_event().build()?;
        let app = NdiClientApp::new(source_selector, synchronize_screens).await?;
        let keep_awake = keep_screen_awake();

        Ok(Self { event_loop, app, keep_awake })
    }

    pub fn create_proxy(&self) -> winit::event_loop::EventLoopProxy<NdiUserEvent> {
        self.event_loop.create_proxy()
    }

    pub fn launch(mut self) -> color_eyre::Result<()> {
        tracing::debug!("launching windows");
        self.event_loop.set_control_flow(ControlFlow::Poll);
        self.event_loop.run_app(&mut self.app)?;

        Ok(())
    }
}

fn keep_screen_awake() -> Option<keep_active::KeepActive> {
    let awake = keep_active::Builder::default()
        .app_name("ndi-client")
        .app_reverse_domain("me.maxjoehnk.ndi-client")
        .display(true)
        .idle(true)
        .sleep(true)
        .create();

    if let Err(err) = awake.as_ref() {
        tracing::warn!("Failed to keep display awake: {err}");
    }

    awake.ok()
}
