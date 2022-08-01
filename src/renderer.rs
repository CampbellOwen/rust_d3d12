use anyhow::{Ok, Result};
use hassle_rs::{compile_hlsl, validate_dxil};
use std::io::Read;

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
                std::ptr::null_mut::<Option<ID3D12Device>>(),
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
) -> Result<ID3D12Device> {
    let mut device: Option<ID3D12Device> = None;
    unsafe { D3D12CreateDevice(adapter, feature_level, &mut device) }?;
    Ok(device.unwrap())
}

fn create_root_signature(device: &ID3D12Device) -> Result<ID3D12RootSignature> {
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

fn create_pipeline_state(
    device: &ID3D12Device,
    root_signature: &ID3D12RootSignature,
) -> Result<ID3D12PipelineState> {
    //let compile_flags = if cfg!(debug_assertions) {
    //    D3DCOMPILE_DEBUG | D3DCOMPILE_SKIP_OPTIMIZATION
    //} else {
    //    0
    //};

    let mut shader_code = String::new();
    std::fs::File::open("src/shaders/triangle.hlsl")?.read_to_string(&mut shader_code)?;

    let vs_ir = compile_hlsl("triangle.hlsl", &shader_code, "VSMain", "vs_6_5", &[], &[])?;
    validate_dxil(&vs_ir)?;

    let ps_ir = compile_hlsl("triangle.hlsl", &shader_code, "PSMain", "ps_6_5", &[], &[])?;
    validate_dxil(&ps_ir)?;

    let mut input_element_descs: [D3D12_INPUT_ELEMENT_DESC; 2] = [
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
            AlignedByteOffset: 12,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];

    let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        InputLayout: D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: input_element_descs.as_mut_ptr(),
            NumElements: input_element_descs.len() as u32,
        },
        pRootSignature: Some(root_signature.clone()),
        VS: D3D12_SHADER_BYTECODE {
            pShaderBytecode: vs_ir.as_ptr() as _,
            BytecodeLength: vs_ir.len(),
        },
        PS: D3D12_SHADER_BYTECODE {
            pShaderBytecode: ps_ir.as_ptr() as _,
            BytecodeLength: ps_ir.len(),
        },
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
    position: [f32; 3],
    color: [f32; 4],
}

fn create_vertex_buffer(
    device: &ID3D12Device,
    aspect_ratio: f32,
) -> Result<(ID3D12Resource, D3D12_VERTEX_BUFFER_VIEW)> {
    let vertices = [
        Vertex {
            position: [0.0, 0.25 * aspect_ratio, 0.0],
            color: [1.0, 0.0, 0.0, 1.0],
        },
        Vertex {
            position: [0.25, -0.25 * aspect_ratio, 0.0],
            color: [0.0, 1.0, 0.0, 1.0],
        },
        Vertex {
            position: [-0.25, -0.25 * aspect_ratio, 0.0],
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

struct SceneResources {
    command_queue: ID3D12CommandQueue,
    swap_chain: IDXGISwapChain3,
    frame_index: u32,
    render_targets: [ID3D12Resource; FRAME_COUNT as usize],
    rtv_heap: ID3D12DescriptorHeap,
    rtv_descriptor_size: usize,
    viewport: D3D12_VIEWPORT,
    scissor_rect: RECT,
    command_allocator: ID3D12CommandAllocator,
    root_signature: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    command_list: ID3D12GraphicsCommandList,

    #[allow(dead_code)]
    vertex_buffer: ID3D12Resource,
    vbv: D3D12_VERTEX_BUFFER_VIEW,
    fence: ID3D12Fence,
    fence_value: u64,
    fence_event: HANDLE,
}

fn bind_to_window(renderer: &Renderer, window_size: (u32, u32)) -> Result<SceneResources> {
    let (width, height) = window_size;
    let command_queue: ID3D12CommandQueue = unsafe {
        renderer
            .device
            .CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
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
        renderer.dxgi_factory.CreateSwapChainForHwnd(
            &command_queue,
            renderer.hwnd,
            &swap_chain_desc,
            std::ptr::null_mut(),
            None,
        )?
    }
    .cast()?;

    unsafe {
        renderer
            .dxgi_factory
            .MakeWindowAssociation(renderer.hwnd, DXGI_MWA_NO_ALT_ENTER)?;
    }

    let frame_index = unsafe { swap_chain.GetCurrentBackBufferIndex() };

    let rtv_heap: ID3D12DescriptorHeap = unsafe {
        renderer
            .device
            .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                NumDescriptors: FRAME_COUNT,
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                ..Default::default()
            })
    }?;

    let rtv_descriptor_size = unsafe {
        renderer
            .device
            .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
    } as usize;
    let rtv_handle = unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };

    let render_targets: [ID3D12Resource; FRAME_COUNT as usize] =
        array_init::try_array_init(|i: usize| -> Result<ID3D12Resource> {
            let render_target: ID3D12Resource = unsafe { swap_chain.GetBuffer(i as u32) }?;
            unsafe {
                renderer.device.CreateRenderTargetView(
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

    let command_allocator: ID3D12CommandAllocator = unsafe {
        renderer
            .device
            .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
    }?;

    let root_signature = create_root_signature(&renderer.device)?;
    let pso = create_pipeline_state(&renderer.device, &root_signature)?;

    let command_list: ID3D12GraphicsCommandList = unsafe {
        renderer.device.CreateCommandList(
            0,
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            &command_allocator,
            &pso,
        )
    }?;
    unsafe {
        command_list.Close()?;
    }

    let aspect_ratio = (width as f32) / (height as f32);

    let (vertex_buffer, vbv) = create_vertex_buffer(&renderer.device, aspect_ratio)?;

    let fence = unsafe { renderer.device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }?;
    let fence_value = 1;
    let fence_event = unsafe { CreateEventA(std::ptr::null(), false, false, None) }?;

    Ok(SceneResources {
        command_queue,
        swap_chain,
        frame_index,
        render_targets,
        rtv_heap,
        rtv_descriptor_size,
        viewport,
        scissor_rect,
        command_allocator,
        root_signature,
        pso,
        command_list,
        vertex_buffer,
        vbv,
        fence,
        fence_value,
        fence_event,
    })
}

fn populate_command_list(resources: &SceneResources) -> Result<()> {
    unsafe {
        resources.command_allocator.Reset()?;
    }

    let command_list = &resources.command_list;
    unsafe {
        command_list.Reset(&resources.command_allocator, &resources.pso)?;
    }

    unsafe {
        command_list.SetGraphicsRootSignature(&resources.root_signature);
        command_list.RSSetViewports(&[resources.viewport]);
        command_list.RSSetScissorRects(&[resources.scissor_rect]);
    }

    let barrier = transition_barrier(
        &resources.render_targets[resources.frame_index as usize],
        D3D12_RESOURCE_STATE_PRESENT,
        D3D12_RESOURCE_STATE_RENDER_TARGET,
    );
    unsafe { command_list.ResourceBarrier(&[barrier]) };

    let rtv_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
        ptr: unsafe { resources.rtv_heap.GetCPUDescriptorHandleForHeapStart() }.ptr
            + resources.frame_index as usize * resources.rtv_descriptor_size,
    };

    unsafe {
        command_list.OMSetRenderTargets(1, &rtv_handle, false, std::ptr::null());
    }

    unsafe {
        command_list.ClearRenderTargetView(rtv_handle, &*[0.0, 0.2, 0.4, 1.0].as_ptr(), &[]);
        command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        command_list.IASetVertexBuffers(0, &[resources.vbv]);
        command_list.DrawInstanced(3, 1, 0, 0);

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
    device: ID3D12Device,
    scene_resources: Option<SceneResources>,
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

        let mut renderer = Renderer {
            hwnd,
            dxgi_factory,
            device,
            scene_resources: None,
        };

        let scene_resources = bind_to_window(&renderer, window_size)?;
        renderer.scene_resources = Some(scene_resources);

        Ok(renderer)
    }
    pub fn render(&mut self) -> Result<()> {
        if let Some(resources) = &mut self.scene_resources {
            populate_command_list(resources)?;

            let command_list = ID3D12CommandList::from(&resources.command_list);
            unsafe {
                resources
                    .command_queue
                    .ExecuteCommandLists(&[Some(command_list)])
            };

            unsafe { resources.swap_chain.Present(1, 0) }.ok()?;

            wait_for_previous_frame(resources)?;
        }

        Ok(())
    }
}

fn wait_for_previous_frame(resources: &mut SceneResources) -> Result<()> {
    let fence = resources.fence_value;
    unsafe { resources.command_queue.Signal(&resources.fence, fence) }?;

    resources.fence_value += 1;

    if unsafe { resources.fence.GetCompletedValue() } < fence {
        unsafe {
            resources
                .fence
                .SetEventOnCompletion(fence, resources.fence_event)
        }?;
        unsafe { WaitForSingleObject(resources.fence_event, INFINITE) };
    }

    resources.frame_index = unsafe { resources.swap_chain.GetCurrentBackBufferIndex() };

    Ok(())
}
