use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use color_eyre::eyre::WrapErr;
use crossbeam_utils::atomic::AtomicCell;
use image::DynamicImage;
use tracing::metadata::LevelFilter;
use tracing_subscriber::{EnvFilter, FmtSubscriber};
use wgpu::PresentMode;
use winit::event::Event;
use winit::event::WindowEvent;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowBuilder, WindowId};

use crate::config::Config;
use crate::image_renderer::WgpuImageRenderer;
use ndi::Source;

use crate::ndi_receiver::*;

// mod gui;
mod config;
mod image_renderer;
mod ndi_receiver;

pub type VideoReceiver = Arc<AtomicCell<Option<DynamicImage>>>;
pub type VideoSender = Arc<AtomicCell<Option<DynamicImage>>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> color_eyre::Result<()> {
    let config = Config::read()?;
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env()?,
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber).context("tracing setup")?;
    tracing::debug!("Initializing NDI");
    ndi::initialize().context("initializing ndi")?;

    let event_loop = winit::event_loop::EventLoop::new()?;

    let mut model = Model::new(config, &event_loop).await?;

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Poll);
        if let Ok(sources) = model.source_recv.try_recv() {
            model.sources = sources;
        }
        model.update();
        match event {
            Event::WindowEvent {
                ref event,
                window_id,
            } => {
                if let Some(screen) = model.screens.get_mut(&window_id) {
                    match event {
                        WindowEvent::KeyboardInput { event, .. }
                            if matches!(event.logical_key, Key::Named(NamedKey::F12)) =>
                        {
                            screen.ui_open = !screen.ui_open;
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if matches!(event.logical_key, Key::Named(NamedKey::Escape)) =>
                        {
                            return;
                        }
                        WindowEvent::Resized(size) => {
                            if size.width > 0 && size.height > 0 {
                                screen
                                    .image_renderer
                                    .resize(&model.device, size.width, size.height)
                            }
                        }
                        WindowEvent::RedrawRequested => {
                            if let Some(screen) = model.screens.get_mut(&window_id) {
                                if let Err(err) =
                                    screen.draw(&model.device, &model.queue, &model.sources)
                                {
                                    tracing::error!(error = ?err, "Failed to draw screen");
                                }
                            };
                        }
                        _ => {}
                    }
                    // screen.egui.handle_event(event);
                }
            }
            _ => {}
        }
    })?;

    Ok(())
}

pub struct Model {
    config: Option<Config>,
    screens: HashMap<WindowId, Screen>,
    sources: Vec<NdiSource>,
    source_recv: Receiver<Vec<NdiSource>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Model {
    async fn new(config: Option<Config>, event_loop: &EventLoop<()>) -> color_eyre::Result<Model> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let mut windows = Vec::new();

        tracing::info!(
            "Available monitors: {:?}",
            event_loop
                .available_monitors()
                .map(|monitor| monitor.name().unwrap_or_default())
                .collect::<Vec<_>>()
        );

        for monitor in event_loop.available_monitors() {
            if let Some(config) = config.as_ref() {
                if !config
                    .screens
                    .iter()
                    .any(|s| s.monitor == monitor.name().unwrap_or_default())
                {
                    continue;
                }
            }
            let window = if let Ok(window) = WindowBuilder::new()
                .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))))
                .build(event_loop) {
                window
            }else {
                WindowBuilder::new().build(event_loop)?
            };

            window.set_title(&format!(
                "NDI Client {}",
                monitor.name().unwrap_or_default()
            ));
            window.set_cursor_visible(false);
            // window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))));
            let surface = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(&window)?)
            }?;

            windows.push((window, surface, monitor));
        }

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: false,
                compatible_surface: windows.first().map(|(_, surface, _)| surface),
                power_preference: wgpu::PowerPreference::HighPerformance,
            })
            .await
            .ok_or_else(|| color_eyre::eyre::eyre!("No compatible video adapter available"))?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    ..Default::default()
                },
                None,
            )
            .await?;
        let (source_tx, source_rx) = std::sync::mpsc::channel();

        std::thread::spawn(|| {
            if let Err(err) = discover_sources(source_tx) {
                tracing::error!(error = ?err, "NDI Source discovery crashed");
            }
        });

        let mut screens = HashMap::new();

        for (window, surface, monitor) in windows {
            let video_queue = Arc::new(AtomicCell::new(None));
            let video_rx = Arc::clone(&video_queue);
            let video_tx = video_queue;
            let (selected_source_tx, selected_source_rx) = std::sync::mpsc::channel();

            std::thread::spawn(|| {
                if let Err(err) = recv_ndi(video_tx, selected_source_rx) {
                    tracing::error!(error = ?err, "NDI Receiver crashed");
                }
            });

            let window_id = window.id();

            // let egui = Egui::from_window(&window).await?;

            let surface_caps = surface.get_capabilities(&adapter);
            let surface_format = surface_caps
                .formats
                .iter()
                .copied()
                .find(|f| f.is_srgb())
                .unwrap_or(surface_caps.formats[0]);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width: window.inner_size().width,
                height: window.inner_size().height,
                present_mode: PresentMode::Immediate,
                alpha_mode: surface_caps.alpha_modes[0],
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&device, &config);

            let image = DynamicImage::new_rgba8(1920, 1080);

            let screen = Screen {
                image: Some(image),
                image_recv: video_rx,
                // egui,
                ui_open: false,
                selected_source: None,
                selected_source_send: selected_source_tx,
                window,
                image_renderer: WgpuImageRenderer::new(&device, surface, config, (1920, 1080))?,
                monitor,
                fullscreen: false,
            };

            screens.insert(window_id, screen);
        }

        Ok(Model {
            config,
            screens,
            source_recv: source_rx,
            sources: Default::default(),
            device,
            queue,
        })
    }

    fn update(&mut self) {
        for screen in self.screens.values_mut() {
            if let Err(err) = screen.update(&self.sources, &self.config) {
                tracing::error!(error = ?err, "Failed to update screen");
            }
        }
    }
}

pub struct Screen {
    image: Option<DynamicImage>,
    image_recv: VideoReceiver,
    selected_source_send: Sender<Source>,
    // egui: Egui,
    ui_open: bool,
    selected_source: Option<Source>,
    window: Window,
    monitor: MonitorHandle,
    image_renderer: WgpuImageRenderer,
    fullscreen: bool,
}

impl Screen {
    fn update(&mut self, sources: &[NdiSource], config: &Option<Config>) -> color_eyre::Result<()> {
        if self.selected_source.is_none() {
            let source = if let Some(config) = config.as_ref().and_then(|config| {
                config
                    .screens
                    .iter()
                    .find(|s| s.monitor == self.monitor.name().unwrap_or_default())
            }) {
                sources
                    .iter()
                    .find(|source| source.name == config.source)
                    .cloned()
            } else if !sources.is_empty() {
                sources.first().cloned()
            } else {
                None
            };
            if let Some(source) = source {
                self.selected_source_send.send(source.source.clone())?;
                self.selected_source = Some(source.source);
            }
        }
        if let Some(image) = self.image_recv.take() {
            self.image = Some(image);
            self.window.request_redraw();
        }
        // gui::update(self, sources);

        Ok(())
    }

    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sources: &[NdiSource],
    ) -> color_eyre::Result<()> {
        if let Some(image) = self.image.as_mut() {
            self.image_renderer.render(device, queue, image)?;
        }
        if self.ui_open {
            // gui::update(self, sources);
        }

        if !self.fullscreen {
            // self.window
            //     .set_fullscreen(Some(Fullscreen::Borderless(Some(self.monitor.clone()))));
            self.fullscreen = true;
        }

        Ok(())
    }
}
