use std::collections::HashMap;
use color_eyre::eyre::Context;
use image::DynamicImage;
use wgpu::{Features, PresentMode};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};
use crate::renderer::image_renderer::WgpuImageRenderer;
use crate::source_selector::{MonitorId, SourceSelector};

pub enum NdiUserEvent {
    ReceivedFrame(MonitorId, DynamicImage),
    RefreshedSources,
}

impl std::fmt::Debug for NdiUserEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NdiUserEvent::ReceivedFrame(monitor_id, _) => write!(f, "ReceivedFrame({:?})", monitor_id),
            NdiUserEvent::RefreshedSources => write!(f, "RefreshedSources"),
        }
    }
}

pub struct NdiClientApp {
    source_selector: SourceSelector,
    monitors: HashMap<MonitorId, WindowId>,
    screens: HashMap<WindowId, Screen>,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl ApplicationHandler<NdiUserEvent> for NdiClientApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        tracing::info!(
            "Available monitors: {:?}",
            event_loop
                .available_monitors()
                .map(|monitor| monitor.name().unwrap_or_default())
                .collect::<Vec<_>>()
        );

        self.launch_monitors(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: NdiUserEvent) {
        tracing::trace!("Received user event: {:?}", event);
        match event {
            NdiUserEvent::ReceivedFrame(monitor_id, image) => {
                if let Some(screen) = self.screens.get_mut(monitor_id.as_ref()) {
                    screen.image = Some(image);
                    screen.window.request_redraw();
                }
            }
            NdiUserEvent::RefreshedSources => self.launch_monitors(event_loop),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::Resized(ref size) => {
                if let Some(screen) = self.screens.get_mut(&window_id) {
                    if size.width > 0 && size.height > 0 {
                        screen
                            .image_renderer
                            .resize(&self.device, size.width, size.height)
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(screen) = self.screens.get_mut(&window_id) {
                    if let Err(err) = screen.draw(&self.device, &self.queue) {
                        tracing::error!(error = ?err, "Failed to draw screen");
                    }

                    tracing::trace!(
                        "FPS: {:>6.2}",
                        1000.0 / screen.last_redraw.elapsed().as_millis() as f64
                    );
                    screen.last_redraw = std::time::Instant::now();
                };
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}

impl NdiClientApp {
    pub async fn new(source_selector: SourceSelector) -> color_eyre::Result<NdiClientApp> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: false,
                compatible_surface: None,
                power_preference: wgpu::PowerPreference::HighPerformance,
            })
            .await
            .context("No compatible video adapter available")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                required_features: Features::PIPELINE_CACHE,
                ..Default::default()
            })
            .await?;

        Ok(NdiClientApp {
            screens: Default::default(),
            monitors: Default::default(),
            source_selector,
            instance,
            adapter,
            device,
            queue,
        })
    }

    fn launch_monitors(&mut self, event_loop: &ActiveEventLoop) {
        tracing::debug!("Launching monitor windows");
        for monitor in event_loop.available_monitors() {
            let monitor_id = MonitorId::from(&monitor);
            tracing::debug!("Checking monitor {monitor_id}");
            if !self.source_selector.is_configured(&monitor_id) {
                tracing::debug!("Monitor {monitor_id:?} not configured, skipping");
                continue;
            }
            if self.monitors.contains_key(&monitor_id) {
                continue;
            }

            self.launch_monitor_window(event_loop, monitor, monitor_id);
        }
    }

    fn launch_monitor_window(&mut self, event_loop: &ActiveEventLoop, monitor: MonitorHandle, monitor_id: MonitorId) {
        tracing::info!("Launching window for monitor {monitor_id:?}");
        let mut attributes = WindowAttributes::default();
        attributes.title = format!("NDI Client {}", monitor_id);
        let mut fullscreen_attributes = attributes.clone();
        fullscreen_attributes.fullscreen = Some(Fullscreen::Borderless(Some(monitor.clone())));
        let window = if let Ok(window) = event_loop.create_window(fullscreen_attributes) {
            window
        } else {
            event_loop.create_window(attributes).unwrap()
        };

        window.set_cursor_visible(false);
        // window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))));
        let surface = unsafe {
            self.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(&window).unwrap())
        }
            .unwrap();

        let window_id = window.id();
        let monitor_id = monitor_id.with_handle(window_id);
        self.source_selector.update_monitor(monitor_id.clone());

        let surface_caps = surface.get_capabilities(&self.adapter);
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
        surface.configure(&self.device, &config);

        let image = DynamicImage::new_rgba8(1920, 1080);

        let screen = Screen {
            image: Some(image),
            window,
            image_renderer: WgpuImageRenderer::new(&self.device, surface, config, (1920, 1080))
                .unwrap(),
            last_redraw: std::time::Instant::now(),
        };

        self.screens.insert(window_id, screen);
        self.monitors.insert(monitor_id, window_id);
    }
}

pub struct Screen {
    image: Option<DynamicImage>,
    window: Window,
    image_renderer: WgpuImageRenderer,
    last_redraw: std::time::Instant,
}

impl Screen {
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> color_eyre::Result<()> {
        if let Some(image) = self.image.as_mut() {
            self.image_renderer.render(device, queue, image)?;
        }

        Ok(())
    }
}
