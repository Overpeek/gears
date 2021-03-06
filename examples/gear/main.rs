use std::{sync::Arc, time::Instant};

use cgmath::{perspective, Deg, InnerSpace, Matrix4, Point3, Rad, Vector3};
use gears::{
    load_obj, Buffer, ContextGPUPick, ContextValidation, EventLoopTarget, Frame, FrameLoop,
    FrameLoopTarget, FramePerfReport, ImmediateFrameInfo, InputState, KeyboardInput, Pipeline,
    RenderRecordInfo, Renderer, RendererRecord, SyncMode, UpdateRecordInfo, VertexBuffer,
    VirtualKeyCode, WindowEvent,
};
use parking_lot::{Mutex, RwLock};

mod shader {
    gears_pipeline::pipeline! {
        vert: { path: "gear/res/default.vert.glsl" }
        frag: { path: "gear/res/default.frag.glsl" }

        builders
    }
}

const MAX_VBO_LEN: usize = 50_000;

struct App {
    frame: Frame,
    renderer: Renderer,
    input: Arc<RwLock<InputState>>,

    vb: VertexBuffer<shader::VertexData>,
    shader: Pipeline,

    delta_time: Mutex<Instant>,
    distance: Mutex<f32>,
    position: Mutex<Vector3<f32>>,
}

impl App {
    fn init(frame: Frame, renderer: Renderer, input: Arc<RwLock<InputState>>) -> Arc<RwLock<Self>> {
        let vb = VertexBuffer::new(&renderer, MAX_VBO_LEN).unwrap();
        let shader = shader::build(&renderer);

        let mut app = Self {
            frame,
            renderer,
            input,

            vb,
            shader,

            delta_time: Mutex::new(Instant::now()),
            distance: Mutex::new(2.5),
            position: Mutex::new(Vector3::new(0.0, 0.0, 0.0)),
        };

        app.reload_mesh();

        Arc::new(RwLock::new(app))
    }

    fn reload_mesh(&mut self) {
        let vertices = load_obj(include_str!("res/gear.obj"), None, |position, normal| {
            shader::VertexData {
                pos: position,
                norm: normal,
            }
        });

        self.vb
            .write(0, &vertices[..vertices.len().min(MAX_VBO_LEN)])
            .unwrap();
    }
}

impl RendererRecord for App {
    fn immediate(&self, imfi: &ImmediateFrameInfo) {
        let dt_s = {
            let mut delta_time = self.delta_time.lock();
            let dt_s = delta_time.elapsed().as_secs_f32();
            *delta_time = Instant::now();
            dt_s
        };
        let aspect = self.frame.aspect();

        let mut distance_delta = 0.0;
        let mut velocity = Vector3::new(0.0, 0.0, 0.0);
        {
            let input = self.input.read();
            if input.key_held(VirtualKeyCode::E) {
                distance_delta += 1.0;
            }
            if input.key_held(VirtualKeyCode::Q) {
                distance_delta -= 1.0;
            }
            if input.key_held(VirtualKeyCode::A) {
                velocity.x += 1.0;
            }
            if input.key_held(VirtualKeyCode::D) {
                velocity.x -= 1.0;
            }
            if input.key_held(VirtualKeyCode::W) {
                velocity.y += 1.0;
            }
            if input.key_held(VirtualKeyCode::S) {
                velocity.y -= 1.0;
            }
            if input.key_held(VirtualKeyCode::Space) {
                velocity.z += 2.0;
            }
        }
        let distance = {
            let mut distance = self.distance.lock();
            *distance += distance_delta * 3.0 * dt_s;
            *distance
        };
        let position = {
            let mut position = self.position.lock();

            *position += velocity * 3.0 * dt_s;
            position.y = position
                .y
                .min(std::f32::consts::PI / 2.0 - 0.0001)
                .max(-std::f32::consts::PI / 2.0 + 0.0001);

            *position
        };

        let eye = Point3::new(
            position.x.sin() * position.y.cos(),
            position.y.sin(),
            position.x.cos() * position.y.cos(),
        ) * distance;
        let focus = Point3::new(0.0, 0.0, 0.0);

        let ubo = shader::UBO {
            model_matrix: Matrix4::from_angle_x(Rad { 0: position.z }),
            view_matrix: Matrix4::look_at_rh(eye, focus, Vector3::new(0.0, -1.0, 0.0)),
            projection_matrix: perspective(Deg { 0: 60.0 }, aspect, 0.01, 100.0),
            light_dir: Vector3::new(0.2, 2.0, 0.5).normalize(),
        };

        self.shader.write_ubo(imfi, &ubo).unwrap();
    }

    fn update(&self, uri: &UpdateRecordInfo) -> bool {
        unsafe { self.shader.update(uri) || self.vb.update(uri) }
    }

    fn record(&self, rri: &RenderRecordInfo) {
        unsafe {
            self.shader.bind(rri);
            self.vb.draw(rri);
        }
    }
}

impl EventLoopTarget for App {
    fn event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput {
                input:
                    KeyboardInput {
                        virtual_keycode: Some(VirtualKeyCode::R),
                        ..
                    },
                ..
            } => {
                self.reload_mesh();
            }
            _ => {}
        }
    }
}

impl FrameLoopTarget for App {
    fn frame(&self) -> FramePerfReport {
        self.renderer.frame(self)
    }
}

fn main() {
    env_logger::init();

    let (frame, event_loop) = Frame::new()
        .with_title("Simple Example")
        .with_size(600, 600)
        .build();

    let context = frame
        .context(ContextGPUPick::Automatic, ContextValidation::WithValidation)
        .unwrap();

    let renderer = Renderer::new()
        .with_sync(SyncMode::Immediate)
        .build(context)
        .unwrap();

    let input = InputState::new();
    let app = App::init(frame, renderer, input.clone());

    FrameLoop::new()
        .with_event_loop(event_loop)
        .with_event_target(input)
        .with_event_target(app.clone())
        .with_frame_target(app)
        .build()
        .run();
}
