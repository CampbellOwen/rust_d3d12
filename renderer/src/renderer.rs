use std::f32::consts::PI;
use std::ffi::c_void;

use anyhow::{ensure, Ok, Result};
use glam::Vec3;
use image::io::Reader as ImageReader;

use windows::core::PCSTR;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

const FRAME_COUNT: usize = 2;

use d3d12_utils::*;

#[allow(dead_code)]
fn load_cube() -> Result<(Vec<ObjVertex>, Vec<u32>)> {
    let cube_obj = std::fs::read_to_string(r"F:\Models\cube.obj")?;

    parse_obj(cube_obj.lines())
}

fn load_bunny() -> Result<(Vec<ObjVertex>, Vec<u32>)> {
    let obj = std::fs::read_to_string(r"F:\Models\bunny.obj")?;

    parse_obj(obj.lines())
}

#[derive(Debug)]
pub(crate) struct RendererResources {
    #[allow(dead_code)]
    hwnd: HWND,
    #[allow(dead_code)]
    dxgi_factory: IDXGIFactory5,
    pub(crate) device: ID3D12Device4,

    graphics_queue: CommandQueue,

    swap_chain: IDXGISwapChain3,
    frame_index: u32,
    back_buffer_handles: [TextureHandle; FRAME_COUNT],
    depth_buffer_handles: [TextureHandle; FRAME_COUNT],
    descriptor_manager: DescriptorManager,
    viewport: D3D12_VIEWPORT,
    scissor_rect: RECT,
    command_allocators: [ID3D12CommandAllocator; FRAME_COUNT as usize],
    root_signature: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    command_list: ID3D12GraphicsCommandList,
    fence_values: [u64; FRAME_COUNT as usize],
    vbv: D3D12_VERTEX_BUFFER_VIEW,
    ibv: D3D12_INDEX_BUFFER_VIEW,

    cbv_descriptors: [DescriptorHandle; FRAME_COUNT as usize],

    upload_ring_buffer: UploadRingBuffer,

    texture_manager: TextureManager,

    #[allow(dead_code)]
    resource_heap: Heap,
    #[allow(dead_code)]
    vertex_buffer: Resource,
    #[allow(dead_code)]
    index_buffer: Resource,

    #[allow(dead_code)]
    texture: TextureHandle,

    #[allow(dead_code)]
    constant_buffers: [Resource; FRAME_COUNT as usize],
}

#[derive(Debug)]
pub struct Renderer {
    pub(crate) resources: Option<RendererResources>,
}

impl Renderer {
    pub fn null() -> Renderer {
        Renderer { resources: None }
    }

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
            device.CheckFeatureSupport(
                D3D12_FEATURE_D3D12_OPTIONS,
                std::ptr::addr_of!(data_options) as *mut c_void,
                std::mem::size_of_val(&data_options) as u32,
            )?;
        }

        let (width, height) = window_size;

        let graphics_queue = CommandQueue::new(&device, D3D12_COMMAND_LIST_TYPE_DIRECT)?;

        let mut upload_ring_buffer = UploadRingBuffer::new(&device, None, None)?;
        let mut texture_manager = TextureManager::new(&device, None)?;
        let mut descriptor_manager = DescriptorManager::new(&device)?;

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
                resource: back_buffer,
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

        let command_allocators: [ID3D12CommandAllocator; FRAME_COUNT as usize] =
            array_init::try_array_init(|_| -> Result<ID3D12CommandAllocator> {
                let allocator =
                    unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }?;
                Ok(allocator)
            })?;

        let root_signature = create_root_signature(&device)?;

        let vertex_shader = compile_vertex_shader("renderer/src/shaders/triangle.hlsl", "VSMain")?;
        let pixel_shader = compile_pixel_shader("renderer/src/shaders/triangle.hlsl", "PSMain")?;

        let input_element_descs: [D3D12_INPUT_ELEMENT_DESC; 3] = [
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(b"POSITION\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32B32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(b"NORMAL\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32B32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 12,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(b"TEXCOORD\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 24,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];
        let pso = create_pipeline_state(
            &device,
            &root_signature,
            &input_element_descs,
            &vertex_shader,
            &pixel_shader,
            1,
        )?;

        let command_list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList1(
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

        let mut resource_heap =
            Heap::create_default_heap(&device, 2e7 as usize, "Scene Resources Heap")?;
        let vertex_buffer = resource_heap.create_resource(
            &device,
            &vb_desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            false,
        )?;

        let vbv = D3D12_VERTEX_BUFFER_VIEW {
            BufferLocation: vertex_buffer.gpu_address(),
            StrideInBytes: std::mem::size_of::<ObjVertex>() as u32,
            SizeInBytes: std::mem::size_of_val(vertices.as_slice()) as u32,
        };

        let upload = upload_ring_buffer.allocate(std::mem::size_of_val(vertices.as_slice()))?;
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

        let index_buffer = resource_heap.create_resource(
            &device,
            &index_buffer_desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            false,
        )?;

        let upload = upload_ring_buffer.allocate(index_buffer_desc.Width as usize)?;
        upload.sub_resource.copy_from(&indices)?;
        upload
            .sub_resource
            .copy_to_resource(&upload.command_list, &index_buffer)?;
        upload.submit(Some(&graphics_queue))?;

        let ibv = D3D12_INDEX_BUFFER_VIEW {
            BufferLocation: index_buffer.gpu_address(),
            SizeInBytes: std::mem::size_of_val(indices.as_slice()) as u32,
            Format: DXGI_FORMAT_R32_UINT,
        };

        // TEXTURE UPLOAD

        let img = ImageReader::open(r"F:\Textures\uv_checker.png")?
            .decode()?
            .to_rgba8();

        let texture = texture_manager.create_texture(
            &device,
            &mut upload_ring_buffer,
            Some(&graphics_queue),
            &mut descriptor_manager,
            TextureInfo {
                dimension: TextureDimension::Two(img.width() as usize, img.height()),
                format: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
                array_size: 1,
                num_mips: 1,
                is_depth_buffer: false,
                is_unordered_access: false,
                is_render_target: false,
            },
            img.as_flat_samples().samples,
        )?;

        let aspect_ratio = (width as f32) / (height as f32);
        let constant_buffer = [
            glam::Mat4::from_translation(Vec3::new(0.0, -0.8, 1.5))
                * glam::Mat4::from_rotation_y(PI),
            glam::Mat4::perspective_lh(PI / 2.0, aspect_ratio, 0.1, 100.0),
        ];
        let constant_buffer_size = align_data(
            std::mem::size_of_val(&constant_buffer),
            D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT as usize,
        );

        let mut cbv_descriptors: [DescriptorHandle; FRAME_COUNT] = Default::default();
        let constant_buffers: [Resource; FRAME_COUNT] = array_init::try_array_init(|i| {
            let buffer = Resource::create_committed(
                &device,
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    ..Default::default()
                },
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Width: constant_buffer_size as u64,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    ..Default::default()
                },
                D3D12_RESOURCE_STATE_GENERIC_READ,
                true,
            )?;

            buffer.copy_from(&constant_buffer)?;

            let cbv_descriptor = descriptor_manager.allocate(DescriptorType::Resource)?;
            cbv_descriptors[i] = cbv_descriptor;

            unsafe {
                device.CreateConstantBufferView(
                    &D3D12_CONSTANT_BUFFER_VIEW_DESC {
                        BufferLocation: buffer.gpu_address(),
                        SizeInBytes: buffer.size as u32,
                    },
                    descriptor_manager.get_cpu_handle(&cbv_descriptor)?,
                )
            };

            Ok(buffer)
        })?;

        let fence_values = [0; 2];
        let resources = RendererResources {
            hwnd,
            dxgi_factory,
            device,

            graphics_queue,
            swap_chain,
            frame_index,
            back_buffer_handles,
            depth_buffer_handles,
            descriptor_manager,
            viewport,
            scissor_rect,
            command_allocators,
            root_signature,
            pso,
            command_list,
            vertex_buffer,
            vbv,
            index_buffer,
            ibv,
            fence_values,

            constant_buffers,
            resource_heap,
            texture,
            texture_manager,
            cbv_descriptors,
            upload_ring_buffer,
        };

        let mut renderer = Renderer {
            resources: Some(resources),
        };

        renderer
            .resources
            .as_mut()
            .unwrap()
            .graphics_queue
            .wait_for_idle()?;

        Ok(renderer)
    }

    fn populate_command_list(&mut self) -> Result<()> {
        ensure!(self.resources.is_some());
        let resources = self.resources.as_mut().unwrap();

        // Resetting the command allocator while the frame is being rendered is not okay
        let command_allocator = &resources.command_allocators[resources.frame_index as usize];
        unsafe {
            command_allocator.Reset()?;
        }

        // Resetting the command list can happen right after submission
        let command_list = &resources.command_list;
        unsafe {
            command_list.Reset(command_allocator, &resources.pso)?;
        }

        let cbv_gpu_handle = resources
            .descriptor_manager
            .get_gpu_handle(&resources.cbv_descriptors[resources.frame_index as usize])?;

        let texture_srv = resources.texture_manager.get_srv(&resources.texture)?;

        let texture_gpu_handle = resources.descriptor_manager.get_gpu_handle(&texture_srv)?;

        unsafe {
            command_list.SetGraphicsRootSignature(&resources.root_signature);

            command_list.SetDescriptorHeaps(&[Some(
                resources
                    .descriptor_manager
                    .get_heap(DescriptorType::Resource)?,
            )]);
            command_list.SetGraphicsRootDescriptorTable(0, cbv_gpu_handle);
            command_list.SetGraphicsRootDescriptorTable(1, texture_gpu_handle);

            command_list.RSSetViewports(&[resources.viewport]);
            command_list.RSSetScissorRects(&[resources.scissor_rect]);
        }

        let render_target_handle = &resources.back_buffer_handles[resources.frame_index as usize];
        let render_target = resources
            .texture_manager
            .get_texture(render_target_handle)?;

        let barrier = transition_barrier(
            &render_target.resource.device_resource,
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );
        unsafe { command_list.ResourceBarrier(&[barrier]) };

        let rtv_handle = resources.texture_manager.get_rtv(render_target_handle)?;
        let rtv = resources.descriptor_manager.get_cpu_handle(&rtv_handle)?;

        let depth_buffer_handle = &resources.depth_buffer_handles[resources.frame_index as usize];
        let dsv_handle = resources.texture_manager.get_dsv(depth_buffer_handle)?;
        let dsv = resources.descriptor_manager.get_cpu_handle(&dsv_handle)?;

        unsafe {
            command_list.OMSetRenderTargets(1, &rtv, false, &dsv);
        }

        unsafe {
            command_list.ClearDepthStencilView(dsv, D3D12_CLEAR_FLAG_DEPTH, 1.0, 0, &[]);
            command_list.ClearRenderTargetView(rtv, &*[0.0, 0.2, 0.4, 1.0].as_ptr(), &[]);
            command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            command_list.IASetVertexBuffers(0, &[resources.vbv]);
            command_list.IASetIndexBuffer(&resources.ibv);
            command_list.DrawIndexedInstanced(432138, 1, 0, 0, 0);

            command_list.ResourceBarrier(&[transition_barrier(
                &render_target.resource.device_resource,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            )]);
        }

        unsafe {
            command_list.Close()?;
        }

        Ok(())
    }

    pub fn resize(&mut self, _extent: (u32, u32)) -> Result<()> {
        ensure!(self.resources.is_some());
        //self.wait_for_idle().expect("All GPU work done");
        //let resources = self.resources.as_mut().unwrap();

        //// Resetting the command allocator while the frame is being rendered is not okay
        //for i in 0..FRAME_COUNT {
        //    let command_allocator = &resources.command_allocators[i];
        //    unsafe {
        //        command_allocator.Reset()?;
        //    }
        //    let command_list = &resources.command_list;
        //    unsafe {
        //        command_list.Close();
        //        command_list.Reset(command_allocator, &resources.pso)?;
        //        command_list.Close();
        //    }
        //}

        //resources.render_targets = Vec::new();

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

        //let rtv_handles: [D3D12_CPU_DESCRIPTOR_HANDLE; FRAME_COUNT] =
        //    array_init::try_array_init(|i| {
        //        let handle = resources.rtv_heap.get_cpu_handle(i as u32)?;
        //        Ok(handle)
        //    })?;
        //let (render_targets, viewport, scissor_rect) = resize_swapchain(
        //    &resources.device,
        //    &resources.swap_chain,
        //    extent,
        //    &rtv_handles,
        //)?;

        //let (width, height) = extent;

        //let depth_buffers: [Tex2D; FRAME_COUNT as usize] = array_init::try_array_init(|i| {
        //    let buffer =
        //        create_depth_stencil_buffer(&resources.device, width as usize, height as usize)?;
        //    let dsv_handle = resources
        //        .dsv_heap
        //        .get_cpu_handle(resources.dsv_indices[i])?;
        //    unsafe {
        //        resources.device.CreateDepthStencilView(
        //            &buffer.resource,
        //            &D3D12_DEPTH_STENCIL_VIEW_DESC {
        //                Format: DXGI_FORMAT_D32_FLOAT,
        //                ViewDimension: D3D12_DSV_DIMENSION_TEXTURE2D,
        //                Flags: D3D12_DSV_FLAG_NONE,
        //                ..Default::default()
        //            },
        //            dsv_handle,
        //        );
        //    }

        //    Ok(buffer)
        //})?;

        //resources.render_targets = render_targets;
        //resources.viewport = viewport;
        //resources.scissor_rect = scissor_rect;
        //resources.depth_buffers = depth_buffers;

        Ok(())
    }

    pub fn wait_for_idle(&mut self) -> Result<()> {
        ensure!(self.resources.is_some());
        let resources = self.resources.as_mut().unwrap();

        for fence in resources.fence_values {
            resources.graphics_queue.wait_for_fence_blocking(fence)?;
        }
        resources.graphics_queue.wait_for_idle()
    }

    pub fn render(&mut self) -> Result<()> {
        ensure!(self.resources.is_some());
        {
            // Let this fall out of scope after waiting to remove the mutable reference
            let resources = self.resources.as_mut().unwrap();

            let last_fence_value = resources.fence_values[resources.frame_index as usize];
            resources
                .graphics_queue
                .wait_for_fence_blocking(last_fence_value)?;
        }

        self.populate_command_list()?;

        let resources = self.resources.as_mut().unwrap();

        let command_list = ID3D12CommandList::from(&resources.command_list);

        let fence_value = resources
            .graphics_queue
            .execute_command_list(&command_list)?;

        resources.fence_values[resources.frame_index as usize] = fence_value;

        unsafe { resources.swap_chain.Present(1, 0) }.ok()?;

        resources.frame_index = unsafe { resources.swap_chain.GetCurrentBackBufferIndex() };

        resources.upload_ring_buffer.clean_up_submissions()?;

        Ok(())
    }
}
