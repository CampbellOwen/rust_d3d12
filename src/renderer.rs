use anyhow::{Context, Ok, Result};
use hassle_rs::{compile_hlsl, validate_dxil};

use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HANDLE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::{CreateEventA, WaitForSingleObject};
use windows::Win32::System::WindowsProgramming::INFINITE;

const FRAME_COUNT: u32 = 2;

fn get_hardware_adapter(
    factory: &IDXGIFactory5,
    feature_level: D3D_FEATURE_LEVEL,
) -> Result<IDXGIAdapter1> {
    for i in 0.. {
        let adapter = unsafe { factory.EnumAdapters1(i)? };
        let desc = unsafe { adapter.GetDesc1()? };

        if (DXGI_ADAPTER_FLAG(desc.Flags) & DXGI_ADAPTER_FLAG_SOFTWARE) != DXGI_ADAPTER_FLAG_NONE {
            continue;
        }

        if unsafe {
            D3D12CreateDevice(
                &adapter,
                feature_level,
                std::ptr::null_mut::<Option<ID3D12Device4>>(),
            )
        }
        .is_ok()
        {
            return Ok(adapter);
        }
    }

    unreachable!()
}

fn create_dxgi_factory() -> Result<IDXGIFactory5> {
    let dxgi_factory_flags = if cfg!(debug_assertions) {
        DXGI_CREATE_FACTORY_DEBUG
    } else {
        0
    };

    let factory = unsafe { CreateDXGIFactory2(dxgi_factory_flags) }?;

    Ok(factory)
}

fn create_device(
    adapter: &IDXGIAdapter1,
    feature_level: D3D_FEATURE_LEVEL,
) -> Result<ID3D12Device4> {
    let mut device: Option<ID3D12Device4> = None;
    unsafe { D3D12CreateDevice(adapter, feature_level, &mut device) }?;
    Ok(device.unwrap())
}

fn create_root_signature(device: &ID3D12Device4) -> Result<ID3D12RootSignature> {
    let desc = D3D12_ROOT_SIGNATURE_DESC {
        Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        ..Default::default()
    };

    let mut signature = None;
    let signature = unsafe {
        D3D12SerializeRootSignature(
            &desc,
            D3D_ROOT_SIGNATURE_VERSION_1,
            &mut signature,
            std::ptr::null_mut(),
        )
    }
    .map(|()| signature.unwrap())?;

    let root_signature = unsafe {
        device.CreateRootSignature(
            0,
            std::slice::from_raw_parts(
                signature.GetBufferPointer() as _,
                signature.GetBufferSize(),
            ),
        )
    }?;

    Ok(root_signature)
}

pub struct CompiledShader {
    pub name: String,
    pub byte_code: Vec<u8>,
}

impl CompiledShader {
    pub fn get_handle(&self) -> D3D12_SHADER_BYTECODE {
        D3D12_SHADER_BYTECODE {
            pShaderBytecode: self.byte_code.as_ptr() as _,
            BytecodeLength: self.byte_code.len(),
        }
    }
}

const SHADER_COMPILE_FLAGS: &[&str] = if cfg!(debug_assertions) {
    &["-Od", "-Zi"]
} else {
    &[]
};

fn compile_shader(filename: &str, entry_point: &str, shader_model: &str) -> Result<CompiledShader> {
    let path = std::path::Path::new(filename);

    let shader_source = std::fs::read_to_string(path)?;
    let name = path
        .file_name()
        .context("No filename")?
        .to_str()
        .map(|str| str.to_string())
        .context("Can't convert to string")?;

    let ir = compile_hlsl(
        &name,
        &shader_source,
        entry_point,
        shader_model,
        SHADER_COMPILE_FLAGS,
        &[],
    )?;
    validate_dxil(&ir)?;

    Ok(CompiledShader {
        name,
        byte_code: ir,
    })
}

pub fn compile_pixel_shader(filename: &str, entry_point: &str) -> Result<CompiledShader> {
    compile_shader(filename, entry_point, "ps_6_5")
}

pub fn compile_vertex_shader(filename: &str, entry_point: &str) -> Result<CompiledShader> {
    compile_shader(filename, entry_point, "vs_6_5")
}

fn create_pipeline_state(
    device: &ID3D12Device4,
    root_signature: &ID3D12RootSignature,
    vertex_shader: &CompiledShader,
    pixel_shader: &CompiledShader,
) -> Result<ID3D12PipelineState> {
    let input_element_descs: [D3D12_INPUT_ELEMENT_DESC; 2] = [
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(b"POSITION\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(b"COLOR\0".as_ptr()),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 16,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];

    let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        InputLayout: D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: input_element_descs.as_ptr(),
            NumElements: input_element_descs.len() as u32,
        },
        pRootSignature: Some(root_signature.clone()),
        VS: vertex_shader.get_handle(),
        PS: pixel_shader.get_handle(),
        RasterizerState: D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            DepthClipEnable: true.into(),
            ..Default::default()
        },
        BlendState: D3D12_BLEND_DESC {
            AlphaToCoverageEnable: false.into(),
            IndependentBlendEnable: false.into(),
            RenderTarget: [
                D3D12_RENDER_TARGET_BLEND_DESC {
                    BlendEnable: false.into(),
                    LogicOpEnable: false.into(),
                    SrcBlend: D3D12_BLEND_ONE,
                    DestBlend: D3D12_BLEND_ZERO,
                    BlendOp: D3D12_BLEND_OP_ADD,
                    SrcBlendAlpha: D3D12_BLEND_ONE,
                    DestBlendAlpha: D3D12_BLEND_ZERO,
                    BlendOpAlpha: D3D12_BLEND_OP_ADD,
                    LogicOp: D3D12_LOGIC_OP_NOOP,
                    RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
                },
                D3D12_RENDER_TARGET_BLEND_DESC::default(),
                D3D12_RENDER_TARGET_BLEND_DESC::default(),
                D3D12_RENDER_TARGET_BLEND_DESC::default(),
                D3D12_RENDER_TARGET_BLEND_DESC::default(),
                D3D12_RENDER_TARGET_BLEND_DESC::default(),
                D3D12_RENDER_TARGET_BLEND_DESC::default(),
                D3D12_RENDER_TARGET_BLEND_DESC::default(),
            ],
        },
        DepthStencilState: D3D12_DEPTH_STENCIL_DESC::default(),
        SampleMask: u32::MAX,
        PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
        NumRenderTargets: 1,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    desc.RTVFormats[0] = DXGI_FORMAT_R8G8B8A8_UNORM;

    let pso = unsafe { device.CreateGraphicsPipelineState(&desc) }?;

    Ok(pso)
}

#[repr(C)]
struct Vertex {
    position: [f32; 4],
    color: [f32; 4],
}

fn create_vertex_buffer(
    device: &ID3D12Device4,
    aspect_ratio: f32,
) -> Result<(ID3D12Resource, D3D12_VERTEX_BUFFER_VIEW)> {
    let vertices = [
        Vertex {
            position: [0.0, 0.25 * aspect_ratio, 0.0, 1.0],
            color: [1.0, 0.0, 0.0, 1.0],
        },
        Vertex {
            position: [0.25, -0.25 * aspect_ratio, 0.0, 1.0],
            color: [0.0, 1.0, 0.0, 1.0],
        },
        Vertex {
            position: [-0.25, -0.25 * aspect_ratio, 0.0, 1.0],
            color: [0.0, 0.0, 1.0, 1.0],
        },
    ];

    let mut vertex_buffer: Option<ID3D12Resource> = None;
    unsafe {
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_UPLOAD,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Width: std::mem::size_of_val(&vertices) as u64,
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
            std::ptr::null(),
            &mut vertex_buffer,
        )
    }?;
    let vertex_buffer = vertex_buffer.unwrap();

    unsafe {
        let mut data = std::ptr::null_mut();
        vertex_buffer.Map(0, std::ptr::null(), &mut data)?;
        std::ptr::copy_nonoverlapping(
            vertices.as_ptr(),
            data as *mut Vertex,
            std::mem::size_of_val(&vertices),
        );
        vertex_buffer.Unmap(0, std::ptr::null());
    }

    let vbv = D3D12_VERTEX_BUFFER_VIEW {
        BufferLocation: unsafe { vertex_buffer.GetGPUVirtualAddress() },
        StrideInBytes: std::mem::size_of::<Vertex>() as u32,
        SizeInBytes: std::mem::size_of_val(&vertices) as u32,
    };

    Ok((vertex_buffer, vbv))
}

fn create_index_buffer(
    device: &ID3D12Device4,
) -> Result<(ID3D12Resource, D3D12_INDEX_BUFFER_VIEW)> {
    let indices = [0, 1, 2];

    let mut index_buffer: Option<ID3D12Resource> = None;
    unsafe {
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_UPLOAD,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Width: std::mem::size_of_val(&indices) as u64,
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
            std::ptr::null(),
            &mut index_buffer,
        )
    }?;

    let index_buffer = index_buffer.unwrap();
    unsafe {
        let mut data = std::ptr::null_mut();
        index_buffer.Map(0, std::ptr::null(), &mut data)?;

        std::ptr::copy_nonoverlapping(
            indices.as_ptr(),
            data as *mut u32,
            std::mem::size_of_val(&indices),
        );

        index_buffer.Unmap(0, std::ptr::null());
    }

    let ibv = D3D12_INDEX_BUFFER_VIEW {
        BufferLocation: unsafe { index_buffer.GetGPUVirtualAddress() },
        SizeInBytes: std::mem::size_of_val(&indices) as u32,
        Format: DXGI_FORMAT_R32_UINT,
    };

    Ok((index_buffer, ibv))
}

fn transition_barrier(
    resource: &ID3D12Resource,
    state_before: D3D12_RESOURCE_STATES,
    state_after: D3D12_RESOURCE_STATES,
) -> D3D12_RESOURCE_BARRIER {
    D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: Some(resource.clone()),
                StateBefore: state_before,
                StateAfter: state_after,
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
            }),
        },
    }
}

pub struct Renderer {
    hwnd: HWND,
    dxgi_factory: IDXGIFactory5,
    device: ID3D12Device4,

    command_queue: ID3D12CommandQueue,
    swap_chain: IDXGISwapChain3,
    frame_index: u32,
    render_targets: [ID3D12Resource; FRAME_COUNT as usize],
    rtv_heap: ID3D12DescriptorHeap,
    rtv_descriptor_size: usize,
    viewport: D3D12_VIEWPORT,
    scissor_rect: RECT,
    command_allocators: [ID3D12CommandAllocator; FRAME_COUNT as usize],
    root_signature: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    command_list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    fence_values: [u64; FRAME_COUNT as usize],
    fence_event: HANDLE,

    vertex_buffer: ID3D12Resource,
    vbv: D3D12_VERTEX_BUFFER_VIEW,

    index_buffer: ID3D12Resource,
    ibv: D3D12_INDEX_BUFFER_VIEW,
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

        let (width, height) = window_size;
        let command_queue: ID3D12CommandQueue = unsafe {
            device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                ..Default::default()
            })
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
                &command_queue,
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

        let rtv_heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                NumDescriptors: FRAME_COUNT,
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                ..Default::default()
            })
        }?;

        let rtv_descriptor_size =
            unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) }
                as usize;
        let rtv_handle = unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };

        let render_targets: [ID3D12Resource; FRAME_COUNT as usize] =
            array_init::try_array_init(|i: usize| -> Result<ID3D12Resource> {
                let render_target: ID3D12Resource = unsafe { swap_chain.GetBuffer(i as u32) }?;
                unsafe {
                    device.CreateRenderTargetView(
                        &render_target,
                        std::ptr::null(),
                        D3D12_CPU_DESCRIPTOR_HANDLE {
                            ptr: rtv_handle.ptr + i * rtv_descriptor_size,
                        },
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

        let vertex_shader = compile_vertex_shader("src/shaders/triangle.hlsl", "VSMain")?;
        let pixel_shader = compile_pixel_shader("src/shaders/triangle.hlsl", "PSMain")?;

        let pso = create_pipeline_state(&device, &root_signature, &vertex_shader, &pixel_shader)?;

        let command_list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList1(
                0,
                D3D12_COMMAND_LIST_TYPE_DIRECT,
                D3D12_COMMAND_LIST_FLAG_NONE,
            )
        }?;

        let aspect_ratio = (width as f32) / (height as f32);

        let (vertex_buffer, vbv) = create_vertex_buffer(&device, aspect_ratio)?;
        let (index_buffer, ibv) = create_index_buffer(&device)?;

        let mut fence_values = [0; 2];

        let fence = unsafe {
            device.CreateFence(fence_values[frame_index as usize], D3D12_FENCE_FLAG_NONE)
        }?;

        fence_values[frame_index as usize] += 1;

        let fence_event = unsafe { CreateEventA(std::ptr::null(), false, false, None) }?;

        let mut renderer = Renderer {
            hwnd,
            dxgi_factory,
            device,

            command_queue,
            swap_chain,
            frame_index,
            render_targets,
            rtv_heap,
            rtv_descriptor_size,
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
            fence,
            fence_values,
            fence_event,
        };

        renderer.wait_for_gpu()?;

        Ok(renderer)
    }

    fn populate_command_list(&self) -> Result<()> {
        let command_allocator = &self.command_allocators[self.frame_index as usize];
        unsafe {
            command_allocator.Reset()?;
        }

        let command_list = &self.command_list;
        unsafe {
            command_list.Reset(command_allocator, &self.pso)?;
        }

        unsafe {
            command_list.SetGraphicsRootSignature(&self.root_signature);
            command_list.RSSetViewports(&[self.viewport]);
            command_list.RSSetScissorRects(&[self.scissor_rect]);
        }

        let barrier = transition_barrier(
            &self.render_targets[self.frame_index as usize],
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );
        unsafe { command_list.ResourceBarrier(&[barrier]) };

        let rtv_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: unsafe { self.rtv_heap.GetCPUDescriptorHandleForHeapStart() }.ptr
                + self.frame_index as usize * self.rtv_descriptor_size,
        };

        unsafe {
            command_list.OMSetRenderTargets(1, &rtv_handle, false, std::ptr::null());
        }

        unsafe {
            command_list.ClearRenderTargetView(rtv_handle, &*[0.0, 0.2, 0.4, 1.0].as_ptr(), &[]);
            command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            command_list.IASetVertexBuffers(0, &[self.vbv]);
            command_list.IASetIndexBuffer(&self.ibv);
            command_list.DrawIndexedInstanced(3, 1, 0, 0, 0);

            command_list.ResourceBarrier(&[transition_barrier(
                &self.render_targets[self.frame_index as usize],
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            )]);
        }

        unsafe {
            command_list.Close()?;
        }

        Ok(())
    }

    fn wait_for_gpu(&mut self) -> Result<()> {
        let fence = &self.fence;
        let frame_index = self.frame_index as usize;
        let fence_value = &mut self.fence_values[frame_index];

        unsafe {
            self.command_queue.Signal(fence, *fence_value)?;

            self.fence
                .SetEventOnCompletion(*fence_value, self.fence_event)?;

            WaitForSingleObject(self.fence_event, INFINITE);
        }

        *fence_value += 1;

        Ok(())
    }

    fn move_to_next_frame(&mut self) -> Result<()> {
        let current_fence_value = self.fence_values[self.frame_index as usize];

        unsafe { self.command_queue.Signal(&self.fence, current_fence_value) }?;

        self.frame_index = unsafe { self.swap_chain.GetCurrentBackBufferIndex() };

        let completed_value = unsafe { self.fence.GetCompletedValue() };
        if completed_value < self.fence_values[self.frame_index as usize] {
            unsafe {
                self.fence.SetEventOnCompletion(
                    self.fence_values[self.frame_index as usize],
                    self.fence_event,
                )?;
                WaitForSingleObject(self.fence_event, INFINITE);
            }
        }
        self.fence_values[self.frame_index as usize] = current_fence_value + 1;

        Ok(())
    }

    pub fn render(&mut self) -> Result<()> {
        self.populate_command_list()?;

        let command_list = ID3D12CommandList::from(&self.command_list);
        unsafe {
            self.command_queue
                .ExecuteCommandLists(&[Some(command_list)])
        };

        unsafe { self.swap_chain.Present(1, 0) }.ok()?;

        self.move_to_next_frame()?;

        Ok(())
    }
}
