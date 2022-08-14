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
    let mut is_closing = false;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => match event {
                WindowEvent::CloseRequested => {
                    is_closing = true;
                    renderer.wait_for_gpu().unwrap();
                    renderer = Renderer::null();
                    *control_flow = ControlFlow::Exit
                }
                WindowEvent::Resized(PhysicalSize { width, height }) => {
                    renderer.resize((width, height))
                }
                _ => (),
            },
            Event::MainEventsCleared => {
                if !is_closing {
                    renderer.render().unwrap()
                }
            }
            _ => (),
        };
    });
}
