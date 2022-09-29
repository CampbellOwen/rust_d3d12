use std::f32::consts::PI;
use std::ffi::c_void;
use std::fs::File;
use std::io::BufReader;

use anyhow::{Context, Ok, Result};
use glam::Vec3;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

const FRAME_COUNT: usize = 2;

use d3d12_utils::*;

use crate::object::Object;
use crate::render_pass::bindless_texture_pass::BindlessTexturePass;

#[allow(dead_code)]
fn load_cube() -> Result<(Vec<ObjVertex>, Vec<u32>)> {
    let cube_obj = std::fs::read_to_string(r"F:\Models\cube.obj")?;

    parse_obj(cube_obj.lines())
}

fn load_bunny() -> Result<(Vec<ObjVertex>, Vec<u32>)> {
    let obj = std::fs::read_to_string(r"assets/bunny.obj")?;

    parse_obj(obj.lines())
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    V: glam::Mat4,
    P: glam::Mat4,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct MaterialConstantBuffer {
    pub texture_index: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct ModelConstantBuffer {
    pub M: glam::Mat4,
}

#[derive(Debug)]
pub struct Resources {
    pub device: ID3D12Device4,
    pub frame_index: u32,
    pub descriptor_manager: DescriptorManager,
    pub texture_manager: TextureManager,
    pub mesh_manager: MeshManager,
    pub upload_ring_buffer: UploadRingBuffer,
    pub viewport: D3D12_VIEWPORT,
    pub scissor_rect: RECT,
    pub camera: Camera,
}
#[derive(Debug)]
pub(crate) struct Renderer {
    #[allow(dead_code)]
    hwnd: HWND,
    #[allow(dead_code)]
    dxgi_factory: IDXGIFactory5,

    command_allocators: [ID3D12CommandAllocator; FRAME_COUNT as usize],
    graphics_queue: CommandQueue,
    swap_chain: IDXGISwapChain3,
    back_buffer_handles: [TextureHandle; FRAME_COUNT],
    depth_buffer_handles: [TextureHandle; FRAME_COUNT],
    command_list: ID3D12GraphicsCommandList,
    fence_values: [u64; FRAME_COUNT as usize],

    pub(crate) resources: Resources,

    basic_render_pass: BindlessTexturePass<FRAME_COUNT>,

    objects: Vec<Object>,
}

#[derive(Debug)]
pub struct Application {
    pub(crate) renderer: Option<Renderer>,
}

static mut COUNTER: u32 = 0;

impl Application {
    pub fn null() -> Application {
        Application { renderer: None }
    }

    pub fn new(hwnd: HWND, window_size: (u32, u32)) -> Result<Application> {
        Ok(Self {
            renderer: Some(Renderer::new(hwnd, window_size)?),
        })
    }

    pub fn render(&mut self) -> Result<()> {
        self.renderer.as_mut().context("No renderer")?.render()
    }

    pub fn resize(&mut self, extent: (u32, u32)) -> Result<()> {
        self.renderer
            .as_mut()
            .context("No renderer")?
            .resize(extent)
    }

    pub fn wait_for_idle(&mut self) -> Result<()> {
        self.renderer
            .as_mut()
            .context("No renderer")?
            .wait_for_idle()
    }
}
impl Renderer {
    pub fn new(hwnd: HWND, window_size: (u32, u32)) -> Result<Renderer> {
        if cfg!(debug_assertions) {
            unsafe {
                let mut debug: Option<ID3D12Debug> = None;
                if let Some(debug) = D3D12GetDebugInterface(&mut debug).ok().and(debug) {
                    debug.EnableDebugLayer();
                }
            }
        }

        let dxgi_factory = create_dxgi_factory()?;

        let feature_level = D3D_FEATURE_LEVEL_11_0;

        let adapter = get_hardware_adapter(&dxgi_factory, feature_level)?;

        let device = create_device(&adapter, feature_level)?;

        let data_options = D3D12_FEATURE_DATA_D3D12_OPTIONS {
            ResourceHeapTier: D3D12_RESOURCE_HEAP_TIER_2,
            ..Default::default()
        };
        unsafe {
            device
                .CheckFeatureSupport(
                    D3D12_FEATURE_D3D12_OPTIONS,
                    std::ptr::addr_of!(data_options) as *mut c_void,
                    std::mem::size_of_val(&data_options) as u32,
                )
                .expect("Feature not supported");
        }

        let (width, height) = window_size;

        let mut graphics_queue = CommandQueue::new(
            &device,
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            "Main Graphics Queue",
        )?;

        let upload_ring_buffer = UploadRingBuffer::new(&device, None, Some(5e8 as usize))?;
        let mut texture_manager = TextureManager::new(&device, None)?;
        let mut descriptor_manager = DescriptorManager::new(&device)?;
        let mesh_manager = MeshManager::new(&device)?;

        let swap_chain_format = DXGI_FORMAT_R8G8B8A8_UNORM;
        let swap_chain = create_swapchain(
            hwnd,
            &dxgi_factory,
            &graphics_queue,
            FRAME_COUNT as u32,
            swap_chain_format,
            (width, height),
        )?;
        let frame_index = unsafe { swap_chain.GetCurrentBackBufferIndex() };
        unsafe {
            dxgi_factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
        }

        let mut back_buffer_handles: [TextureHandle; FRAME_COUNT] = Default::default();
        let mut depth_buffer_handles: [TextureHandle; FRAME_COUNT] = Default::default();
        for i in 0..FRAME_COUNT {
            let back_buffer: ID3D12Resource = unsafe { swap_chain.GetBuffer(i as u32) }?;
            unsafe {
                back_buffer.SetName(PCWSTR::from(&format!("Backbuffer {}", COUNTER).into()))?;
                COUNTER += 1;
            }
            let back_buffer = Resource {
                device_resource: back_buffer,
                size: (width * height * 4) as usize,
                mapped_data: std::ptr::null_mut(),
            };
            let back_buffer = Texture {
                info: TextureInfo {
                    dimension: TextureDimension::Two(width as usize, height),
                    format: swap_chain_format,
                    array_size: 1,
                    num_mips: 1,
                    is_render_target: true,
                    is_depth_buffer: false,
                    is_unordered_access: false,
                },
                resource: Some(back_buffer),
            };

            back_buffer_handles[i] =
                texture_manager.add_texture(&device, &mut descriptor_manager, back_buffer)?;

            depth_buffer_handles[i] = texture_manager.create_empty_texture(
                &device,
                TextureInfo {
                    dimension: TextureDimension::Two(width as usize, height),
                    format: DXGI_FORMAT_D32_FLOAT,
                    array_size: 1,
                    num_mips: 1,
                    is_render_target: false,
                    is_depth_buffer: true,
                    is_unordered_access: false,
                },
                Some(D3D12_CLEAR_VALUE {
                    Format: DXGI_FORMAT_D32_FLOAT,
                    Anonymous: D3D12_CLEAR_VALUE_0 {
                        DepthStencil: D3D12_DEPTH_STENCIL_VALUE {
                            Depth: 1.0,
                            Stencil: 0,
                        },
                    },
                }),
                D3D12_RESOURCE_STATE_DEPTH_WRITE,
                &mut descriptor_manager,
                true,
            )?;
        }

        let viewport = D3D12_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: width as f32,
            Height: height as f32,
            MinDepth: D3D12_MIN_DEPTH,
            MaxDepth: D3D12_MAX_DEPTH,
        };

        let scissor_rect = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };

        let aspect_ratio = (width as f32) / (height as f32);
        let camera = Camera {
            V: glam::Mat4::from_translation(Vec3::new(0.0, -0.8, 1.5)).inverse(),
            P: glam::Mat4::perspective_lh(PI / 2.0, aspect_ratio, 0.1, 100.0),
        };
        let mut resources = Resources {
            device,
            frame_index,
            descriptor_manager,
            texture_manager,
            mesh_manager,
            upload_ring_buffer,
            viewport,
            scissor_rect,
            camera,
        };

        let command_allocators: [ID3D12CommandAllocator; FRAME_COUNT as usize] =
            array_init::try_array_init(|_| -> Result<ID3D12CommandAllocator> {
                let allocator = unsafe {
                    resources
                        .device
                        .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                }?;
                Ok(allocator)
            })?;

        let command_list: ID3D12GraphicsCommandList = unsafe {
            resources.device.CreateCommandList1(
                0,
                D3D12_COMMAND_LIST_TYPE_DIRECT,
                D3D12_COMMAND_LIST_FLAG_NONE,
            )
        }?;

        let (vertices, indices) = load_bunny()?;

        let vb_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: std::mem::size_of_val(vertices.as_slice()) as u64,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            ..Default::default()
        };

        let vertex_buffer = resources.mesh_manager.heap.create_resource(
            &resources.device,
            &vb_desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            false,
        )?;

        let upload = resources
            .upload_ring_buffer
            .allocate(std::mem::size_of_val(vertices.as_slice()))?;
        upload.sub_resource.copy_from(&vertices)?;
        upload
            .sub_resource
            .copy_to_resource(&upload.command_list, &vertex_buffer)?;
        upload.submit(Some(&graphics_queue))?;

        let index_buffer_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: std::mem::size_of_val(indices.as_slice()) as u64,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            ..Default::default()
        };

        let index_buffer = resources.mesh_manager.heap.create_resource(
            &resources.device,
            &index_buffer_desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            false,
        )?;

        let upload = resources
            .upload_ring_buffer
            .allocate(index_buffer_desc.Width as usize)?;
        upload.sub_resource.copy_from(&indices)?;
        upload
            .sub_resource
            .copy_to_resource(&upload.command_list, &index_buffer)?;
        upload.submit(Some(&graphics_queue))?;

        // TEXTURE UPLOAD

        let f = File::open(r"assets/uv_checker.dds")?;
        let reader = BufReader::new(f);

        let dds_file = ddsfile::Dds::read(reader)?;

        let dimension = if dds_file.get_depth() > 1 {
            TextureDimension::Three(
                dds_file.get_width() as usize,
                dds_file.get_height(),
                dds_file.get_depth() as u16,
            )
        } else if dds_file.get_height() > 1 {
            TextureDimension::Two(dds_file.get_width() as usize, dds_file.get_height())
        } else {
            TextureDimension::One(dds_file.get_width() as usize)
        };

        let texture_info = TextureInfo {
            dimension,
            format: DXGI_FORMAT(dds_file.get_dxgi_format().context("No DXGI format")? as u32),
            array_size: dds_file.get_num_array_layers() as u16,
            num_mips: dds_file.get_num_mipmap_levels() as u16,
            is_render_target: false,
            is_depth_buffer: false,
            is_unordered_access: false,
        };

        let texture = resources.texture_manager.create_texture(
            &resources.device,
            &mut resources.upload_ring_buffer,
            Some(&graphics_queue),
            &mut resources.descriptor_manager,
            texture_info,
            &dds_file.data,
        )?;

        let mesh_handle = resources.mesh_manager.add(
            vertex_buffer,
            index_buffer,
            std::mem::size_of::<ObjVertex>() as u32,
            vertices.len(),
        )?;

        let objects = vec![
            Object {
                position: Vec3::new(0.0, 0.0, 1.0),
                texture: texture.clone(),
                mesh: mesh_handle,
            },
            //Object {
            //    position: Vec3::new(0.0, 1.0, 0.0),
            //    texture,
            //    mesh: mesh_handle,
            //},
        ];

        graphics_queue.wait_for_idle()?;

        let basic_render_pass = BindlessTexturePass::new(&mut resources)?;

        let fence_values = [0; 2];

        let renderer = Renderer {
            hwnd,
            dxgi_factory,

            resources,

            graphics_queue,
            swap_chain,
            back_buffer_handles,
            depth_buffer_handles,
            command_allocators,
            command_list,
            fence_values,

            basic_render_pass,
            objects,
        };

        Ok(renderer)
    }

    pub fn resize(&mut self, _extent: (u32, u32)) -> Result<()> {
        self.wait_for_idle().expect("All GPU work done");

        // Resetting the command allocator while the frame is being rendered is not okay
        for i in 0..FRAME_COUNT {
            let command_allocator = &self.command_allocators[i];
            unsafe {
                command_allocator.Reset()?;
            }
            let command_list = &self.command_list;
            unsafe {
                command_list.Reset(command_allocator, None)?;
                command_list.Close()?;
            }
            self.command_list = unsafe {
                self.resources.device.CreateCommandList1(
                    0,
                    D3D12_COMMAND_LIST_TYPE_DIRECT,
                    D3D12_COMMAND_LIST_FLAG_NONE,
                )
            }?;
        }

        let (width, height) = _extent;

        //if cfg!(debug_assertions) {
        //    if let std::result::Result::Ok(debug_interface) =
        //        unsafe { DXGIGetDebugInterface1::<IDXGIDebug1>(0) }
        //    {
        //        unsafe {
        //            debug_interface
        //                .ReportLiveObjects(
        //                    DXGI_DEBUG_ALL,
        //                    DXGI_DEBUG_RLO_DETAIL | DXGI_DEBUG_RLO_IGNORE_INTERNAL,
        //                )
        //                .expect("Report live objects")
        //        };
        //    }
        //}

        for i in 0..FRAME_COUNT {
            self.resources.texture_manager.delete(
                &mut self.resources.descriptor_manager,
                self.back_buffer_handles[i].clone(),
            );
            self.back_buffer_handles[i] = Default::default();

            self.resources.texture_manager.delete(
                &mut self.resources.descriptor_manager,
                self.depth_buffer_handles[i].clone(),
            );
            self.depth_buffer_handles[i] = Default::default();
        }

        if cfg!(debug_assertions) {
            if let std::result::Result::Ok(debug_interface) =
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

        unsafe {
            self.swap_chain.ResizeBuffers(
                FRAME_COUNT as u32,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                0,
            )?;
        }

        for i in 0..FRAME_COUNT {
            let back_buffer: ID3D12Resource = unsafe { self.swap_chain.GetBuffer(i as u32) }?;
            unsafe {
                back_buffer.SetName(PCWSTR::from(&format!("Backbuffer {}", COUNTER).into()))?;
                COUNTER += 1;
            }
            let back_buffer = Resource {
                device_resource: back_buffer,
                size: (width * height * 4) as usize,
                mapped_data: std::ptr::null_mut(),
            };
            let back_buffer = Texture {
                info: TextureInfo {
                    dimension: TextureDimension::Two(width as usize, height),
                    format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    array_size: 1,
                    num_mips: 1,
                    is_render_target: true,
                    is_depth_buffer: false,
                    is_unordered_access: false,
                },
                resource: Some(back_buffer),
            };

            self.back_buffer_handles[i] = self.resources.texture_manager.add_texture(
                &self.resources.device,
                &mut self.resources.descriptor_manager,
                back_buffer,
            )?;

            self.depth_buffer_handles[i] = self.resources.texture_manager.create_empty_texture(
                &self.resources.device,
                TextureInfo {
                    dimension: TextureDimension::Two(width as usize, height),
                    format: DXGI_FORMAT_D32_FLOAT,
                    array_size: 1,
                    num_mips: 1,
                    is_render_target: false,
                    is_depth_buffer: true,
                    is_unordered_access: false,
                },
                Some(D3D12_CLEAR_VALUE {
                    Format: DXGI_FORMAT_D32_FLOAT,
                    Anonymous: D3D12_CLEAR_VALUE_0 {
                        DepthStencil: D3D12_DEPTH_STENCIL_VALUE {
                            Depth: 1.0,
                            Stencil: 0,
                        },
                    },
                }),
                D3D12_RESOURCE_STATE_DEPTH_WRITE,
                &mut self.resources.descriptor_manager,
                true,
            )?;
        }

        self.resources.frame_index = unsafe { self.swap_chain.GetCurrentBackBufferIndex() };

        self.resources.viewport = D3D12_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: width as f32,
            Height: height as f32,
            MinDepth: D3D12_MIN_DEPTH,
            MaxDepth: D3D12_MAX_DEPTH,
        };

        self.resources.scissor_rect = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };

        let aspect_ratio = (width as f32) / (height as f32);

        let camera = Camera {
            V: glam::Mat4::from_translation(Vec3::new(0.0, -0.8, 1.5)),
            P: glam::Mat4::perspective_lh(PI / 2.0, aspect_ratio, 0.1, 100.0),
        };

        self.resources.camera = camera;

        Ok(())
    }

    pub fn wait_for_idle(&mut self) -> Result<()> {
        for fence in self.fence_values {
            self.graphics_queue.wait_for_fence_blocking(fence)?;
        }
        self.graphics_queue.wait_for_idle()
    }

    pub fn render(&mut self) -> Result<()> {
        let last_fence_value = self.fence_values[self.resources.frame_index as usize];
        self.graphics_queue
            .wait_for_fence_blocking(last_fence_value)?;

        //self.populate_command_list()?;
        // Resetting the command allocator while the frame is being rendered is not okay
        let command_allocator = &self.command_allocators[self.resources.frame_index as usize];
        unsafe {
            command_allocator.Reset()?;
        }

        // Resetting the command list can happen right after submission
        let command_list = &self.command_list;
        unsafe {
            command_list.Reset(command_allocator, None)?;
        }

        let render_target_handle = &self.back_buffer_handles[self.resources.frame_index as usize];
        let depth_buffer_handle = &self.depth_buffer_handles[self.resources.frame_index as usize];

        let rtv_handle = self
            .resources
            .texture_manager
            .get_rtv(render_target_handle)?;
        let rtv = self
            .resources
            .descriptor_manager
            .get_cpu_handle(&rtv_handle)?;

        let dsv_handle = self
            .resources
            .texture_manager
            .get_dsv(depth_buffer_handle)?;
        let dsv = self
            .resources
            .descriptor_manager
            .get_cpu_handle(&dsv_handle)?;
        unsafe {
            command_list.ClearDepthStencilView(dsv, D3D12_CLEAR_FLAG_DEPTH, 1.0, 0, &[]);
            command_list.ClearRenderTargetView(rtv, &*[0.0, 0.2, 0.4, 1.0].as_ptr(), &[]);
        }

        let render_target = self
            .resources
            .texture_manager
            .get_texture(render_target_handle)?;

        let barrier = transition_barrier(
            &render_target.get_resource()?.device_resource,
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );
        unsafe { command_list.ResourceBarrier(&[barrier.clone()]) };

        let _: D3D12_RESOURCE_TRANSITION_BARRIER =
            unsafe { std::mem::ManuallyDrop::into_inner(barrier.Anonymous.Transition) };
        self.basic_render_pass.render(
            command_list,
            &mut self.resources,
            render_target_handle,
            depth_buffer_handle,
            &self.objects,
        )?;

        unsafe {
            command_list.Close()?;
        }

        let generic_command_list = ID3D12CommandList::from(&self.command_list);

        let fence_value = self
            .graphics_queue
            .execute_command_list(&generic_command_list)?;

        self.fence_values[self.resources.frame_index as usize] = fence_value;

        let render_target = self
            .resources
            .texture_manager
            .get_texture(render_target_handle)?;

        unsafe {
            let barrier = transition_barrier(
                &render_target.get_resource()?.device_resource,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            );
            command_list.ResourceBarrier(&[barrier.clone()]);
            let _: D3D12_RESOURCE_TRANSITION_BARRIER =
                std::mem::ManuallyDrop::into_inner(barrier.Anonymous.Transition);
        }

        unsafe { self.swap_chain.Present(1, 0) }.ok()?;

        self.resources.frame_index = unsafe { self.swap_chain.GetCurrentBackBufferIndex() };

        self.resources.upload_ring_buffer.clean_up_submissions()?;

        Ok(())
    }
}
