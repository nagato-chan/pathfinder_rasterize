#[macro_use]
extern crate log;

use egl::Api;
use egl::Instance;
use image::RgbaImage;
use khronos_egl as egl;
use khronos_egl::COLORSPACE_sRGB;
use libloading::Library;
use pathfinder_color::ColorF;
use pathfinder_geometry::{
    rect::{RectF, RectI},
    transform2d::Transform2F,
    vector::{Vector2F, Vector2I},
};
use pathfinder_gl::{GLDevice, GLVersion};
use pathfinder_gpu::{Device, RenderTarget, TextureData, TextureFormat};
use pathfinder_renderer::{
    concurrent::rayon::RayonExecutor,
    gpu::{
        options::{DestFramebuffer, RendererLevel, RendererMode, RendererOptions},
        renderer::Renderer,
    },
    options::{BuildOptions, RenderTransform},
    scene::Scene,
};
use pathfinder_resources::embedded::EmbeddedResourceLoader;
use std::thread::sleep;
use std::time::Duration;
use surfman::{Connection, ContextAttributeFlags, ContextAttributes, GLVersion as SGLVersion};

mod gl {
    include!(concat!(env!("OUT_DIR"), "/gl_bindings.rs"));
}
#[cfg(not(target_os = "android"))]
pub(crate) use crate::gl::Gl;
#[cfg(target_os = "android")]
pub(crate) use crate::gl::Gles2 as Gl;
pub struct Rasterizer {
    egl: Instance<egl::Dynamic<Library, egl::EGL1_5>>,
    display: egl::Display,
    surface: egl::Surface,
    context: egl::Context,
    renderer: Option<(Renderer<GLDevice>, Vector2I)>,
    render_level: RendererLevel,
}

impl Rasterizer {
    pub fn new() -> Self {
        Rasterizer::new_with_level(RendererLevel::D3D9)
    }
    pub fn new_with_level(render_level: RendererLevel) -> Self {
        let egl = unsafe {
            // CHANGED
            egl::DynamicInstance::<egl::EGL1_5>::load_required().expect("unable to load libEGL.so")
        };

        let display = egl.get_display(egl::DEFAULT_DISPLAY).expect("display");
        let (major, minor) = egl.initialize(display).expect("init");
        debug!("egl version: {}", egl.version());
        let attrib_list = [
            egl::SURFACE_TYPE,
            egl::PBUFFER_BIT,
            egl::BLUE_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::RED_SIZE,
            8,
            egl::DEPTH_SIZE,
            8,
            egl::RENDERABLE_TYPE,
            // CONDITION CHANGE
            egl::OPENGL_ES3_BIT,
            egl::NONE,
        ];

        let config = egl
            .choose_first_config(display, &attrib_list)
            .expect("unable to choose config")
            .expect("unable to get first config");
        info!("config: {:?}", config);
        let pbuffer_attrib_list = [egl::NONE];
        let surface = egl
            .create_pbuffer_surface(display, config, &pbuffer_attrib_list)
            .expect("cannot create surface");

        egl.bind_api(egl::OPENGL_ES_API)
            .expect("unable to select OpenGL API");

        let context = egl
            .create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE],
            )
            .expect("cannot create context");

        egl.make_current(display, Some(surface), Some(surface), Some(context))
            .expect("cannot set up current");
        info!("Setting Up OpenGL");
        // Setup Open GL

        Gl::load_with(|name| {
            trace!("{:?}", name);
            egl.get_proc_address(name)
                .expect("failed to create a process") as *const std::ffi::c_void
        });
        info!("Finished Loading");
        Rasterizer {
            egl,
            display,
            surface,
            context,
            renderer: None,
            render_level,
        }
    }

    fn make_current(&self) {
        self.egl
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .unwrap();
    }

    fn renderer_for_size(&mut self, size: Vector2I) -> &mut Renderer<GLDevice> {
        // let conn = Connection::new().unwrap();
        // let adapter = conn.create_adapter().unwrap();
        // let mut dev = conn.create_device(&adapter).unwrap();
        // let attr = ContextAttributes {
        //     version: SGLVersion::new(3, 0),
        //     flags: ContextAttributeFlags::ALPHA,
        // };
        // info!("{:?}", dev.gl_api());
        // let ctx = unsafe {
        //     dev.create_context_from_native_context(surfman::NativeContext {
        //         egl_context: self.context.as_ptr(),
        //         egl_read_surface: self.surface.as_ptr(),
        //         egl_draw_surface: self.surface.as_ptr(),
        //     })
        //     .unwrap()
        // };
        // let fb = dev
        //     .context_surface_info(&ctx)
        //     .unwrap()
        //     .unwrap()
        //     .framebuffer_object;

        info!("check renderer");

        let level = self.render_level;
        let size = Vector2I::new((size.x() + 15) & !15, (size.y() + 15) & !15);
        let (ref mut renderer, ref mut current_size) = *self.renderer.get_or_insert_with(|| {
            let resource_loader = EmbeddedResourceLoader::new();

            let renderer_gl_version = match level {
                RendererLevel::D3D9 => GLVersion::GLES3,
                RendererLevel::D3D11 => GLVersion::GL4,
            };

            info!("get gl device");
            sleep(Duration::from_secs(5));
            info!("getting..");
            let device = GLDevice::new(renderer_gl_version, 0);
            info!("creating texture");
            let tex = device.create_texture(TextureFormat::RGBA8, size);
            info!("creating framebuffer");
            let fb = device.create_framebuffer(tex);
            let dest = DestFramebuffer::Other(fb);
            let render_options = RendererOptions {
                dest,
                background_color: None,
                show_debug_ui: false,
            };
            info!("setting up renderer");
            let renderer = Renderer::new(
                device,
                &resource_loader,
                RendererMode { level },
                render_options,
            );
            (renderer, size)
        });

        if size != *current_size {
            let tex = renderer.device().create_texture(TextureFormat::RGBA8, size);
            let fb = renderer.device().create_framebuffer(tex);
            let dest = DestFramebuffer::Other(fb);
            renderer.options_mut().dest = dest;
            *current_size = size;
        }

        renderer
    }

    pub fn rasterize(&mut self, mut scene: Scene, background: Option<ColorF>) -> RgbaImage {
        self.make_current();

        let view_box = dbg!(scene.view_box());
        let size = view_box.size().ceil().to_i32();
        let transform = Transform2F::from_translation(-view_box.origin());
        info!("initialize rendering");
        let renderer = self.renderer_for_size(size);
        renderer.options_mut().background_color = background;
        scene.set_view_box(RectF::new(Vector2F::zero(), view_box.size()));

        let options = BuildOptions {
            transform: RenderTransform::Transform2D(transform),
            dilation: Vector2F::default(),
            subpixel_aa_enabled: false,
        };

        scene.build_and_render(renderer, options, RayonExecutor);
        // panic!()
        let render_target = match renderer.options().dest {
            DestFramebuffer::Other(ref fb) => RenderTarget::Framebuffer(fb),
            _ => todo!(),
        };
        let texture_data_receiver = renderer
            .device()
            .read_pixels(&render_target, RectI::new(Vector2I::zero(), size));
        let pixels = match renderer.device().recv_texture_data(&texture_data_receiver) {
            TextureData::U8(pixels) => pixels,
            _ => todo!("Unexpected pixel format for default framebuffer!"),
        };

        RgbaImage::from_raw(size.x() as u32, size.y() as u32, pixels)
            .expect("cannot transform into image")
    }
}

impl Drop for Rasterizer {
    fn drop(&mut self) {
        self.egl.terminate(self.display).unwrap();
    }
}

#[test]
fn test_render() {
    use pathfinder_geometry::rect::RectF;

    let mut scene = Scene::new();
    scene.set_view_box(RectF::new(Vector2F::zero(), Vector2F::new(100., 100.)));
    Rasterizer::new().rasterize(scene, None);
}
