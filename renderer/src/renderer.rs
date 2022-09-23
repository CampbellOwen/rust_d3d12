use std::f32::consts::PI;
use std::ffi::c_void;
use std::fs::File;
use std::io::BufReader;

use anyhow::{ensure, Context, Ok, Result};
use glam::Vec3;

use windows::core::{PCSTR, PCWSTR};
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
    root_signature: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    command_list: ID3D12GraphicsCommandList,
    fence_values: [u64; FRAME_COUNT as usize],

    pub(crate) resources: Resources,

    mesh_handles: Vec<MeshHandle>,

    texture: TextureHandle,

    #[allow(dead_code)]
    camera_constant_buffers: [Resource; FRAME_COUNT as usize],
    camera_cbv_descriptors: [DescriptorHandle; FRAME_COUNT as usize],
    #[allow(dead_code)]
    material_constant_buffers: [Resource; FRAME_COUNT as usize],
    material_descriptors: [DescriptorHandle; FRAME_COUNT as usize],
    #[allow(dead_code)]
    model_constant_buffers: [Resource; FRAME_COUNT as usize],
    model_descriptors: [DescriptorHandle; FRAME_COUNT as usize],
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

        let graphics_queue = CommandQueue::new(
            &device,
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            "Main Graphics Queue",
        )?;

        let mut upload_ring_buffer = UploadRingBuffer::new(&device, None, Some(5e8 as usize))?;
        let mut texture_manager = TextureManager::new(&device, None)?;
        let mut descriptor_manager = DescriptorManager::new(&device)?;
        let mut mesh_manager = MeshManager::new(&device)?;

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

        let command_allocators: [ID3D12CommandAllocator; FRAME_COUNT as usize] =
            array_init::try_array_init(|_| -> Result<ID3D12CommandAllocator> {
                let allocator =
                    unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }?;
                Ok(allocator)
            })?;

        let root_signature = create_root_signature(&device)?;

        let vertex_shader =
            compile_vertex_shader("renderer/src/shaders/bindless_texture.hlsl", "VSMain")?;
        let pixel_shader =
            compile_pixel_shader("renderer/src/shaders/bindless_texture.hlsl", "PSMain")?;

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

        let vertex_buffer = mesh_manager.heap.create_resource(
            &device,
            &vb_desc,
            D3D12_RESOURCE_STATE_COMMON,
            None,
            false,
        )?;

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

        let index_buffer = mesh_manager.heap.create_resource(
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

        let mesh_handles = vec![mesh_manager.add(
            vertex_buffer,
            index_buffer,
            std::mem::size_of::<ObjVertex>() as u32,
            vertices.len(),
        )?];

        // TEXTURE UPLOAD

        let f = File::open(r"F:\Textures\uv_checker_8k_colour.dds")?;
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

        let texture = texture_manager.create_texture(
            &device,
            &mut upload_ring_buffer,
            Some(&graphics_queue),
            &mut descriptor_manager,
            texture_info,
            &dds_file.data,
        )?;

        let aspect_ratio = (width as f32) / (height as f32);
        let camera_cb = [
            glam::Mat4::from_translation(Vec3::new(0.0, -0.8, 1.5)).inverse(),
            //* glam::Mat4::from_rotation_y(PI),
            glam::Mat4::perspective_lh(PI / 2.0, aspect_ratio, 0.1, 100.0),
        ];
        let camera_buffer_size = align_data(
            std::mem::size_of_val(&camera_cb),
            D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT as usize,
        );

        let mut camera_cbv_descriptors: [DescriptorHandle; FRAME_COUNT] = Default::default();
        let camera_constant_buffers: [Resource; FRAME_COUNT] = array_init::try_array_init(|i| {
            let buffer = Resource::create_committed(
                &device,
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    ..Default::default()
                },
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Width: camera_buffer_size as u64,
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
                None,
                true,
            )?;

            buffer.copy_from(&camera_cb)?;

            let cbv_descriptor = descriptor_manager.allocate(DescriptorType::Resource)?;
            camera_cbv_descriptors[i] = cbv_descriptor;

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

        let srv = texture_manager.get_srv(&texture)?;

        let material_data = MaterialConstantBuffer {
            texture_index: srv.index as u32,
        };
        let material_buffer_size = align_data(
            std::mem::size_of_val(&material_data),
            D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT as usize,
        );
        let mut material_descriptors: [DescriptorHandle; FRAME_COUNT] = Default::default();
        let material_constant_buffers: [Resource; FRAME_COUNT] = array_init::try_array_init(|i| {
            let buffer = Resource::create_committed(
                &device,
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    ..Default::default()
                },
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Width: material_buffer_size as u64,
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
                None,
                true,
            )?;

            buffer.copy_from(&[material_data])?;

            let cbv_descriptor = descriptor_manager.allocate(DescriptorType::Resource)?;
            material_descriptors[i] = cbv_descriptor;

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

        let model_data = ModelConstantBuffer {
            M: glam::Mat4::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
        };
        let model_buffer_size = align_data(
            std::mem::size_of_val(&model_data),
            D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT as usize,
        );
        let mut model_descriptors: [DescriptorHandle; FRAME_COUNT] = Default::default();
        let model_constant_buffers: [Resource; FRAME_COUNT] = array_init::try_array_init(|i| {
            let buffer = Resource::create_committed(
                &device,
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    ..Default::default()
                },
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Width: model_buffer_size as u64,
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
                None,
                true,
            )?;

            buffer.copy_from(&[model_data])?;

            let cbv_descriptor = descriptor_manager.allocate(DescriptorType::Resource)?;
            model_descriptors[i] = cbv_descriptor;

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
        let resources = Renderer {
            hwnd,
            dxgi_factory,

            resources: Resources {
                device,
                frame_index,
                descriptor_manager,
                texture_manager,
                mesh_manager,
                upload_ring_buffer,
                viewport,
                scissor_rect,
            },

            graphics_queue,
            swap_chain,
            back_buffer_handles,
            depth_buffer_handles,
            command_allocators,
            root_signature,
            pso,
            command_list,
            fence_values,

            camera_constant_buffers,
            camera_cbv_descriptors,
            material_constant_buffers,
            material_descriptors,
            model_constant_buffers,
            model_descriptors,

            mesh_handles,

            texture,
        };

        let mut renderer = Application {
            renderer: Some(resources),
        };

        renderer
            .renderer
            .as_mut()
            .unwrap()
            .graphics_queue
            .wait_for_idle()?;

        Ok(renderer)
    }

    fn populate_command_list(&mut self) -> Result<()> {
        ensure!(self.renderer.is_some());
        let internal_resources = self.renderer.as_mut().unwrap();

        // Resetting the command allocator while the frame is being rendered is not okay
        let command_allocator = &internal_resources.command_allocators
            [internal_resources.resources.frame_index as usize];
        unsafe {
            command_allocator.Reset()?;
        }

        // Resetting the command list can happen right after submission
        let command_list = &internal_resources.command_list;
        unsafe {
            command_list.Reset(command_allocator, &internal_resources.pso)?;
        }

        let camera_cb_handle = internal_resources
            .resources
            .descriptor_manager
            .get_gpu_handle(
                &internal_resources.camera_cbv_descriptors
                    [internal_resources.resources.frame_index as usize],
            )?;

        let model_cb_handle = internal_resources
            .resources
            .descriptor_manager
            .get_gpu_handle(
                &internal_resources.model_descriptors
                    [internal_resources.resources.frame_index as usize],
            )?;

        let material_cb_handle = internal_resources
            .resources
            .descriptor_manager
            .get_gpu_handle(
                &internal_resources.material_descriptors
                    [internal_resources.resources.frame_index as usize],
            )?;

        unsafe {
            command_list.SetDescriptorHeaps(&[Some(
                internal_resources
                    .resources
                    .descriptor_manager
                    .get_heap(DescriptorType::Resource)?,
            )]);
            command_list.SetGraphicsRootSignature(&internal_resources.root_signature);

            command_list.SetGraphicsRootDescriptorTable(0, camera_cb_handle);
            command_list.SetGraphicsRootDescriptorTable(1, material_cb_handle);
            command_list.SetGraphicsRootDescriptorTable(2, model_cb_handle);

            command_list.RSSetViewports(&[internal_resources.resources.viewport]);
            command_list.RSSetScissorRects(&[internal_resources.resources.scissor_rect]);
        }

        let render_target_handle = &internal_resources.back_buffer_handles
            [internal_resources.resources.frame_index as usize];
        let render_target = internal_resources
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

        let rtv_handle = internal_resources
            .resources
            .texture_manager
            .get_rtv(render_target_handle)?;
        let rtv = internal_resources
            .resources
            .descriptor_manager
            .get_cpu_handle(&rtv_handle)?;

        let depth_buffer_handle = &internal_resources.depth_buffer_handles
            [internal_resources.resources.frame_index as usize];
        let dsv_handle = internal_resources
            .resources
            .texture_manager
            .get_dsv(depth_buffer_handle)?;
        let dsv = internal_resources
            .resources
            .descriptor_manager
            .get_cpu_handle(&dsv_handle)?;

        unsafe {
            command_list.OMSetRenderTargets(1, &rtv, false, &dsv);
        }

        for mesh in &internal_resources.mesh_handles {
            unsafe {
                command_list.ClearDepthStencilView(dsv, D3D12_CLEAR_FLAG_DEPTH, 1.0, 0, &[]);
                command_list.ClearRenderTargetView(rtv, &*[0.0, 0.2, 0.4, 1.0].as_ptr(), &[]);
                command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                command_list.IASetVertexBuffers(0, &[mesh.vbv.context("No vertex buffer view")?]);
                command_list.IASetIndexBuffer(&mesh.ibv.context("No index buffer view")?);
                command_list.DrawIndexedInstanced(432138, 10, 0, 0, 0);
            }
        }
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

        unsafe {
            command_list.Close()?;
        }

        Ok(())
    }

    pub fn resize(&mut self, _extent: (u32, u32)) -> Result<()> {
        ensure!(self.renderer.is_some());
        self.wait_for_idle().expect("All GPU work done");
        let resources = self.renderer.as_mut().unwrap();

        // Resetting the command allocator while the frame is being rendered is not okay
        for i in 0..FRAME_COUNT {
            let command_allocator = &resources.command_allocators[i];
            unsafe {
                command_allocator.Reset()?;
            }
            let command_list = &resources.command_list;
            unsafe {
                command_list.Reset(command_allocator, &resources.pso)?;
                command_list.Close()?;
            }
            resources.command_list = unsafe {
                resources.resources.device.CreateCommandList1(
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
            resources.resources.texture_manager.delete(
                &mut resources.resources.descriptor_manager,
                resources.back_buffer_handles[i].clone(),
            );
            resources.back_buffer_handles[i] = Default::default();

            resources.resources.texture_manager.delete(
                &mut resources.resources.descriptor_manager,
                resources.depth_buffer_handles[i].clone(),
            );
            resources.depth_buffer_handles[i] = Default::default();
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

        std::println!("RESIZING BACKBUFFERS to ({},{})", width, height);

        unsafe {
            resources.swap_chain.ResizeBuffers(
                FRAME_COUNT as u32,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                0,
            )?;
        }

        for i in 0..FRAME_COUNT {
            let back_buffer: ID3D12Resource = unsafe { resources.swap_chain.GetBuffer(i as u32) }?;
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

            resources.back_buffer_handles[i] = resources.resources.texture_manager.add_texture(
                &resources.resources.device,
                &mut resources.resources.descriptor_manager,
                back_buffer,
            )?;

            resources.depth_buffer_handles[i] =
                resources.resources.texture_manager.create_empty_texture(
                    &resources.resources.device,
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
                    &mut resources.resources.descriptor_manager,
                    true,
                )?;
        }

        resources.resources.frame_index =
            unsafe { resources.swap_chain.GetCurrentBackBufferIndex() };

        resources.resources.viewport = D3D12_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: width as f32,
            Height: height as f32,
            MinDepth: D3D12_MIN_DEPTH,
            MaxDepth: D3D12_MAX_DEPTH,
        };

        resources.resources.scissor_rect = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };

        let aspect_ratio = (width as f32) / (height as f32);
        let constant_buffer = [
            glam::Mat4::from_translation(Vec3::new(0.0, -0.8, 1.5))
                * glam::Mat4::from_rotation_y(PI),
            glam::Mat4::perspective_lh(PI / 2.0, aspect_ratio, 0.1, 100.0),
        ];

        for cb in &mut resources.camera_constant_buffers {
            cb.copy_from(&constant_buffer)?;
        }

        Ok(())
    }

    pub fn wait_for_idle(&mut self) -> Result<()> {
        ensure!(self.renderer.is_some());
        let resources = self.renderer.as_mut().unwrap();

        for fence in resources.fence_values {
            resources.graphics_queue.wait_for_fence_blocking(fence)?;
        }
        resources.graphics_queue.wait_for_idle()
    }

    pub fn render(&mut self) -> Result<()> {
        ensure!(self.renderer.is_some());
        {
            // Let this fall out of scope after waiting to remove the mutable reference
            let resources = self.renderer.as_mut().unwrap();

            let last_fence_value = resources.fence_values[resources.resources.frame_index as usize];
            resources
                .graphics_queue
                .wait_for_fence_blocking(last_fence_value)?;
        }

        self.populate_command_list()?;

        let resources = self.renderer.as_mut().unwrap();

        let command_list = ID3D12CommandList::from(&resources.command_list);

        let fence_value = resources
            .graphics_queue
            .execute_command_list(&command_list)?;

        resources.fence_values[resources.resources.frame_index as usize] = fence_value;

        unsafe { resources.swap_chain.Present(1, 0) }.ok()?;

        resources.resources.frame_index =
            unsafe { resources.swap_chain.GetCurrentBackBufferIndex() };

        resources
            .resources
            .upload_ring_buffer
            .clean_up_submissions()?;

        Ok(())
    }
}
