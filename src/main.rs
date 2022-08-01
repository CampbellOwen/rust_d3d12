use windows::Win32::Foundation::HWND;
use winit::{
    dpi::PhysicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    platform::windows::WindowExtWindows,
    window::WindowBuilder,
};

mod renderer;
use renderer::Renderer;

fn main() {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().build(&event_loop).unwrap();

    let hwnd = HWND(window.hwnd());

    let PhysicalSize { width, height } = window.inner_size();
    let mut renderer = Renderer::new(hwnd, (width, height)).unwrap();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                _ => (),
            },
            Event::MainEventsCleared => renderer.render().unwrap(),
            _ => (),
        };
    });
}
