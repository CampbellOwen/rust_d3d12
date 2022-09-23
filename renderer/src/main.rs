use windows::Win32::{Foundation::HWND, Graphics::Dxgi::*};
use winit::{
    dpi::{LogicalSize, PhysicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    platform::windows::WindowExtWindows,
    window::WindowBuilder,
};

mod renderer;
use renderer::Application;

mod object;
mod render_pass;

fn main() {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_inner_size(LogicalSize {
            width: 1920,
            height: 1080,
        })
        .build(&event_loop)
        .unwrap();

    let hwnd = HWND(window.hwnd());

    let PhysicalSize {
        mut width,
        mut height,
    } = window.inner_size();
    let mut application = Application::new(hwnd, (width, height)).unwrap();
    let mut is_closing = false;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => match event {
                WindowEvent::CloseRequested => {
                    is_closing = true;

                    if cfg!(debug_assertions) {
                        if let Ok(debug_interface) =
                            unsafe { DXGIGetDebugInterface1::<IDXGIDebug1>(0) }
                        {
                            unsafe {
                                debug_interface
                                    .ReportLiveObjects(
                                        DXGI_DEBUG_ALL,
                                        DXGI_DEBUG_RLO_DETAIL | DXGI_DEBUG_RLO_IGNORE_INTERNAL,
                                    )
                                    .expect("Report live objects")
                            };
                        }
                    }

                    application.wait_for_idle().unwrap();
                    application = Application::null();
                    *control_flow = ControlFlow::Exit
                }
                WindowEvent::Resized(PhysicalSize {
                    width: w,
                    height: h,
                }) => {
                    if w != width || h != height {
                        application
                            .resize((width, height))
                            .expect("Resizing should not fail");

                        width = w;
                        height = h;
                    }
                }
                _ => (),
            },
            Event::MainEventsCleared => {
                if !is_closing {
                    let res = application.render();
                    if res.is_err() && application.renderer.is_some() {
                        unsafe {
                            application
                                .renderer
                                .as_ref()
                                .unwrap()
                                .resources
                                .device
                                .GetDeviceRemovedReason()
                                .unwrap()
                        };
                    }
                }
            }
            _ => (),
        };
    });
}
