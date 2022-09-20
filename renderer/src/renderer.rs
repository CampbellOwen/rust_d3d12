use std::f32::consts::PI;

use anyhow::{ensure, Ok, Result};
use glam::Vec3;

use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

const FRAME_COUNT: u32 = 2;

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
struct RendererResources {
    #[allow(dead_code)]
    hwnd: HWND,
    #[allow(dead_code)]
    dxgi_factory: IDXGIFactory5,
    #[allow(dead_code)]
    device: ID3D12Device4,

    graphics_queue: CommandQueue,

    #[allow(dead_code)]
    copy_queue: CommandQueue,
    #[allow(dead_code)]
    copy_command_allocator: ID3D12CommandAllocator,
    #[allow(dead_code)]
    copy_command_list: ID3D12GraphicsCommandList,

    swap_chain: IDXGISwapChain3,
    frame_index: u32,
    render_targets: [ID3D12Resource; FRAME_COUNT as usize],
    rtv_heap: DescriptorHeap,
    cbv_heap: DescriptorHeap,
    dsv_heap: DescriptorHeap,
    viewport: D3D12_VIEWPORT,
    scissor_rect: RECT,
    command_allocators: [ID3D12CommandAllocator; FRAME_COUNT as usize],
    root_signature: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    command_list: ID3D12GraphicsCommandList,
    fence_values: [u64; FRAME_COUNT as usize],
    vbv: D3D12_VERTEX_BUFFER_VIEW,
    ibv: D3D12_INDEX_BUFFER_VIEW,

    #[allow(dead_code)]
    vertex_buffer_heap: Heap,
    #[allow(dead_code)]
    vertex_buffer: Resource,
    #[allow(dead_code)]
    index_buffer: Resource,
    #[allow(dead_code)]
    constant_buffers: [Resource; FRAME_COUNT as usize],
    #[allow(dead_code)]
    depth_buffers: [Tex2D; FRAME_COUNT as usize],
}

#[derive(Debug)]
pub struct Renderer {
    resources: Option<RendererResources>,
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

        let mut copy_queue = CommandQueue::new(&device, D3D12_COMMAND_LIST_TYPE_COPY)?;
        let copy_command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_COPY) }?;
        unsafe {
            copy_command_allocator.Reset()?;
        }
        let copy_command_list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList1(
                0,
                D3D12_COMMAND_LIST_TYPE_COPY,
                D3D12_COMMAND_LIST_FLAG_NONE,
            )
        }?;

        let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
            BufferCount: FRAME_COUNT,
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                ..Default::default()
            },
            ..Default::default()
        };

        let swap_chain: IDXGISwapChain3 = unsafe {
            dxgi_factory.CreateSwapChainForHwnd(
                &graphics_queue.queue,
                hwnd,
                &swap_chain_desc,
                std::ptr::null_mut(),
                None,
            )?
        }
        .cast()?;

        unsafe {
            dxgi_factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
        }

        let frame_index = unsafe { swap_chain.GetCurrentBackBufferIndex() };

        let mut rtv_heap = DescriptorHeap::render_target_view_heap(&device, FRAME_COUNT)?;

        let render_targets: [ID3D12Resource; FRAME_COUNT as usize] =
            array_init::try_array_init(|i: usize| -> Result<ID3D12Resource> {
                let render_target: ID3D12Resource = unsafe { swap_chain.GetBuffer(i as u32) }?;
                unsafe {
                    device.CreateRenderTargetView(
                        &render_target,
                        std::ptr::null(),
                        rtv_heap.allocate_handle()?,
                    )
                };
                Ok(render_target)
            })?;

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

        let mut dsv_heap = DescriptorHeap::depth_stencil_view_heap(&device, 2)?;
        let depth_buffers: [Tex2D; FRAME_COUNT as usize] = array_init::try_array_init(|_| {
            let buffer = create_depth_stencil_buffer(&device, width as usize, height as usize)?;

            unsafe {
                device.CreateDepthStencilView(
                    &buffer.resource,
                    &D3D12_DEPTH_STENCIL_VIEW_DESC {
                        Format: DXGI_FORMAT_D32_FLOAT,
                        ViewDimension: D3D12_DSV_DIMENSION_TEXTURE2D,
                        Flags: D3D12_DSV_FLAG_NONE,
                        ..Default::default()
                    },
                    dsv_heap.allocate_handle()?,
                );
            }

            Ok(buffer)
        })?;

        let command_list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList1(
                0,
                D3D12_COMMAND_LIST_TYPE_DIRECT,
                D3D12_COMMAND_LIST_FLAG_NONE,
            )
        }?;

        let mut upload_heap = Heap::create_upload_heap(&device, 155569680)?;
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

        let vertex_buffer_staging = upload_heap.create_resource(
            &device,
            &vb_desc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            true,
        )?;
        vertex_buffer_staging.copy_from(&vertices)?;

        let mut vertex_buffer_heap = Heap::create_default_heap(&device, 155569680)?;
        let vertex_buffer = vertex_buffer_heap.create_resource(
            &device,
            &vb_desc,
            D3D12_RESOURCE_STATE_COMMON,
            false,
        )?;

        let vbv = D3D12_VERTEX_BUFFER_VIEW {
            BufferLocation: vertex_buffer.gpu_address(),
            StrideInBytes: std::mem::size_of::<ObjVertex>() as u32,
            SizeInBytes: std::mem::size_of_val(vertices.as_slice()) as u32,
        };

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
        let index_buffer_staging = upload_heap.create_resource(
            &device,
            &index_buffer_desc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            true,
        )?;
        index_buffer_staging.copy_from(&indices)?;

        let index_buffer = vertex_buffer_heap.create_resource(
            &device,
            &index_buffer_desc,
            D3D12_RESOURCE_STATE_COMMON,
            false,
        )?;

        let ibv = D3D12_INDEX_BUFFER_VIEW {
            BufferLocation: index_buffer.gpu_address(),
            SizeInBytes: std::mem::size_of_val(indices.as_slice()) as u32,
            Format: DXGI_FORMAT_R32_UINT,
        };

        unsafe {
            copy_command_list.Reset(&copy_command_allocator, None)?;
            copy_command_list.CopyResource(
                &vertex_buffer.device_resource,
                &vertex_buffer_staging.device_resource,
            );
            copy_command_list.CopyResource(
                &index_buffer.device_resource,
                &index_buffer_staging.device_resource,
            );
            copy_command_list.Close()?;
        };

        let copy_fence = copy_queue.execute_command_list(&copy_command_list.clone().into())?;
        graphics_queue.insert_wait_for_queue_fence(&copy_queue, copy_fence)?;

        let mut cbv_heap = DescriptorHeap::constant_buffer_view_heap(&device, FRAME_COUNT)?;

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

        let constant_buffers: [Resource; FRAME_COUNT as usize] =
            array_init::try_array_init(|_| {
                //let buffer = create_constant_buffer(&device, constant_buffer_size)?;
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

                unsafe {
                    device.CreateConstantBufferView(
                        &D3D12_CONSTANT_BUFFER_VIEW_DESC {
                            BufferLocation: buffer.gpu_address(),
                            SizeInBytes: buffer.size as u32,
                        },
                        cbv_heap.allocate_handle()?,
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
            copy_queue,
            swap_chain,
            frame_index,
            render_targets,
            rtv_heap,
            cbv_heap,
            dsv_heap,
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
            depth_buffers,
            copy_command_allocator,
            copy_command_list,
            vertex_buffer_heap,
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

    fn populate_command_list(&self) -> Result<()> {
        ensure!(self.resources.is_some());
        let resources = self.resources.as_ref().unwrap();

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

        let cbv_gpu_handle = resources.cbv_heap.get_gpu_handle(resources.frame_index)?;

        unsafe {
            command_list.SetGraphicsRootSignature(&resources.root_signature);

            command_list.SetDescriptorHeaps(&[Some(resources.cbv_heap.heap.clone())]);
            command_list.SetGraphicsRootDescriptorTable(0, cbv_gpu_handle);

            command_list.RSSetViewports(&[resources.viewport]);
            command_list.RSSetScissorRects(&[resources.scissor_rect]);
        }

        let barrier = transition_barrier(
            &resources.render_targets[resources.frame_index as usize],
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );
        unsafe { command_list.ResourceBarrier(&[barrier]) };

        let rtv_handle = resources.rtv_heap.get_cpu_handle(resources.frame_index)?;

        let dsv_handle = resources.dsv_heap.get_cpu_handle(resources.frame_index)?;
        unsafe {
            command_list.OMSetRenderTargets(1, &rtv_handle, false, &dsv_handle);
        }

        unsafe {
            command_list.ClearDepthStencilView(dsv_handle, D3D12_CLEAR_FLAG_DEPTH, 1.0, 0, &[]);
            command_list.ClearRenderTargetView(rtv_handle, &*[0.0, 0.2, 0.4, 1.0].as_ptr(), &[]);
            command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            command_list.IASetVertexBuffers(0, &[resources.vbv]);
            command_list.IASetIndexBuffer(&resources.ibv);
            command_list.DrawIndexedInstanced(432138, 1, 0, 0, 0);

            command_list.ResourceBarrier(&[transition_barrier(
                &resources.render_targets[resources.frame_index as usize],
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            )]);
        }

        unsafe {
            command_list.Close()?;
        }

        Ok(())
    }

    pub fn resize(&mut self, _size: (u32, u32)) {

        // TODO: Implement this
    }

    pub fn wait_for_idle(&mut self) -> Result<()> {
        ensure!(self.resources.is_some());
        let resources = self.resources.as_mut().unwrap();
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

        Ok(())
    }
}
