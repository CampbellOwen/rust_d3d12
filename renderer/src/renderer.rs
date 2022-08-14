use std::f32::consts::PI;
use std::ffi::c_void;

use anyhow::{ensure, Context, Ok, Result};
use glam::Vec3;
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

use crate::parse_obj::{parse_obj, ObjVertex};

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
    let descriptor_ranges = [D3D12_DESCRIPTOR_RANGE {
        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_CBV,
        NumDescriptors: 1,
        BaseShaderRegister: 0,
        RegisterSpace: 0,
        OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
    }];

    let root_parameters = [D3D12_ROOT_PARAMETER {
        ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
        ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
        Anonymous: D3D12_ROOT_PARAMETER_0 {
            DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                NumDescriptorRanges: 1,
                pDescriptorRanges: descriptor_ranges.as_ptr(),
            },
        },
    }];

    let desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 1,
        pParameters: root_parameters.as_ptr(),
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

fn load_cube() -> Result<(Vec<ObjVertex>, Vec<u32>)> {
    let cube_obj = std::fs::read_to_string(r"F:\Models\cube.obj")?;

    parse_obj(cube_obj.lines())
}

fn load_bunny() -> Result<(Vec<ObjVertex>, Vec<u32>)> {
    let obj = std::fs::read_to_string(r"F:\Models\bunny.obj")?;

    parse_obj(obj.lines())
}

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

    let stencil_op = D3D12_DEPTH_STENCILOP_DESC {
        StencilFailOp: D3D12_STENCIL_OP_KEEP,
        StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
        StencilPassOp: D3D12_STENCIL_OP_KEEP,
        StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
    };
    let depth_stencil_desc = D3D12_DEPTH_STENCIL_DESC {
        DepthEnable: true.into(),
        DepthWriteMask: D3D12_DEPTH_WRITE_MASK_ALL,
        DepthFunc: D3D12_COMPARISON_FUNC_LESS,
        StencilEnable: false.into(),
        FrontFace: stencil_op,
        BackFace: stencil_op,
        StencilReadMask: D3D12_DEFAULT_STENCIL_READ_MASK as u8,
        StencilWriteMask: D3D12_DEFAULT_STENCIL_READ_MASK as u8,
    };

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
        DepthStencilState: depth_stencil_desc,
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

fn create_vertex_buffer<T: Sized + std::fmt::Debug>(
    device: &ID3D12Device4,
    vertices: &[T],
) -> Result<(ID3D12Resource, D3D12_VERTEX_BUFFER_VIEW)> {
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
                Width: std::mem::size_of_val(vertices) as u64,
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
            vertices.as_ptr() as *mut u8,
            data as *mut u8,
            std::mem::size_of_val(vertices),
        );
        vertex_buffer.Unmap(0, std::ptr::null());
    }

    let vbv = D3D12_VERTEX_BUFFER_VIEW {
        BufferLocation: unsafe { vertex_buffer.GetGPUVirtualAddress() },
        StrideInBytes: std::mem::size_of::<Vertex>() as u32,
        SizeInBytes: std::mem::size_of_val(vertices) as u32,
    };

    Ok((vertex_buffer, vbv))
}

fn create_index_buffer(
    device: &ID3D12Device4,
    indices: &[u32],
) -> Result<(ID3D12Resource, D3D12_INDEX_BUFFER_VIEW)> {
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
                Width: std::mem::size_of_val(indices) as u64,
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
            indices.as_ptr() as *mut u8,
            data as *mut u8,
            std::mem::size_of_val(indices),
        );

        index_buffer.Unmap(0, std::ptr::null());
    }

    let ibv = D3D12_INDEX_BUFFER_VIEW {
        BufferLocation: unsafe { index_buffer.GetGPUVirtualAddress() },
        SizeInBytes: std::mem::size_of_val(indices) as u32,
        Format: DXGI_FORMAT_R32_UINT,
    };

    Ok((index_buffer, ibv))
}

fn align_data(location: usize, alignment: usize) -> usize {
    if alignment == 0 || (alignment & (alignment - 1) != 0) {
        panic!("Non power of 2 alignment");
    }

    (location + (alignment - 1)) & !(alignment - 1)
}

#[derive(Debug)]
struct MappedBuffer {
    buffer: ID3D12Resource,
    size: usize,
    data: *mut c_void,
}

#[derive(Debug)]
struct Tex2D {
    resource: ID3D12Resource,
    width: usize,
    height: usize,
}

fn create_depth_stencil_buffer(
    device: &ID3D12Device4,
    width: usize,
    height: usize,
) -> Result<Tex2D> {
    let mut depth_buffer: Option<ID3D12Resource> = None;

    unsafe {
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Width: width as u64,
                Height: height as u32,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_D32_FLOAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Flags: D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL,
                ..Default::default()
            },
            D3D12_RESOURCE_STATE_DEPTH_WRITE,
            &D3D12_CLEAR_VALUE {
                Format: DXGI_FORMAT_D32_FLOAT,
                Anonymous: D3D12_CLEAR_VALUE_0 {
                    DepthStencil: D3D12_DEPTH_STENCIL_VALUE {
                        Depth: 1.0,
                        Stencil: 0,
                    },
                },
            },
            &mut depth_buffer,
        )?
    };
    let depth_buffer = depth_buffer.unwrap();

    Ok(Tex2D {
        resource: depth_buffer,
        width,
        height,
    })
}

fn create_constant_buffer(device: &ID3D12Device4, size: usize) -> Result<MappedBuffer> {
    let mut constant_buffer: Option<ID3D12Resource> = None;
    unsafe {
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_UPLOAD,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Width: size as u64,
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
            &mut constant_buffer,
        )?;
    }
    let constant_buffer = constant_buffer.unwrap();

    let mut p_data = std::ptr::null_mut();
    unsafe { constant_buffer.Map(0, std::ptr::null(), &mut p_data)? };

    Ok(MappedBuffer {
        buffer: constant_buffer,
        size,
        data: p_data,
    })
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

#[derive(Debug)]
pub struct DescriptorHeap {
    heap: ID3D12DescriptorHeap,
    descriptor_size: usize,
    num_descriptors: u32,

    num_allocated: u32,
}

impl DescriptorHeap {
    fn create_heap(
        device: &ID3D12Device4,
        num_descriptors: u32,
        heap_type: D3D12_DESCRIPTOR_HEAP_TYPE,
        flags: D3D12_DESCRIPTOR_HEAP_FLAGS,
    ) -> Result<DescriptorHeap> {
        let heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                NumDescriptors: num_descriptors,
                Type: heap_type,
                Flags: flags,
                ..Default::default()
            })
        }?;

        let rtv_descriptor_size =
            unsafe { device.GetDescriptorHandleIncrementSize(heap_type) } as usize;

        Ok(DescriptorHeap {
            heap,
            descriptor_size: rtv_descriptor_size,
            num_descriptors,
            num_allocated: 0,
        })
    }

    pub fn constant_buffer_view_heap(
        device: &ID3D12Device4,
        num_descriptors: u32,
    ) -> Result<DescriptorHeap> {
        Self::create_heap(
            device,
            num_descriptors,
            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
        )
    }

    pub fn render_target_view_heap(
        device: &ID3D12Device4,
        num_descriptors: u32,
    ) -> Result<DescriptorHeap> {
        Self::create_heap(
            device,
            num_descriptors,
            D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
        )
    }

    pub fn depth_stencil_view_heap(
        device: &ID3D12Device4,
        num_descriptors: u32,
    ) -> Result<DescriptorHeap> {
        Self::create_heap(
            device,
            num_descriptors,
            D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
            D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
        )
    }

    pub fn allocate_handle(&mut self) -> Result<D3D12_CPU_DESCRIPTOR_HANDLE> {
        anyhow::ensure!(
            self.num_allocated < self.num_descriptors,
            "Not enough descriptors"
        );

        let heap_start_handle = unsafe { self.heap.GetCPUDescriptorHandleForHeapStart() };
        let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: heap_start_handle.ptr + self.num_allocated as usize * self.descriptor_size,
        };

        self.num_allocated += 1;

        Ok(handle)
    }

    pub fn get_cpu_handle(&self, index: u32) -> Result<D3D12_CPU_DESCRIPTOR_HANDLE> {
        anyhow::ensure!(index < self.num_allocated, "index out of bounds");

        let heap_start_handle = unsafe { self.heap.GetCPUDescriptorHandleForHeapStart() };
        Ok(D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: heap_start_handle.ptr + (index as usize * self.descriptor_size),
        })
    }

    pub fn get_gpu_handle(&self, index: u32) -> Result<D3D12_GPU_DESCRIPTOR_HANDLE> {
        anyhow::ensure!(index < self.num_allocated, "index out of bounds");

        let heap_start_handle = unsafe { self.heap.GetGPUDescriptorHandleForHeapStart() };
        Ok(D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: heap_start_handle.ptr + (index as u64 * self.descriptor_size as u64),
        })
    }
}

#[derive(Debug)]
struct RendererResources {
    #[allow(dead_code)]
    hwnd: HWND,
    #[allow(dead_code)]
    dxgi_factory: IDXGIFactory5,
    #[allow(dead_code)]
    device: ID3D12Device4,

    command_queue: ID3D12CommandQueue,
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
    fence: ID3D12Fence,
    fence_values: [u64; FRAME_COUNT as usize],
    fence_event: HANDLE,
    vbv: D3D12_VERTEX_BUFFER_VIEW,
    ibv: D3D12_INDEX_BUFFER_VIEW,
    #[allow(dead_code)]
    vertex_buffer: ID3D12Resource,
    #[allow(dead_code)]
    index_buffer: ID3D12Resource,
    #[allow(dead_code)]
    constant_buffers: [MappedBuffer; FRAME_COUNT as usize],
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

        let pso = create_pipeline_state(&device, &root_signature, &vertex_shader, &pixel_shader)?;

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

        let aspect_ratio = (width as f32) / (height as f32);

        //let (vertices, indices) = load_cube()?;
        let (vertices, indices) = load_bunny()?;

        let (vertex_buffer, vbv) = create_vertex_buffer(&device, &vertices)?;

        let (index_buffer, ibv) = create_index_buffer(&device, &indices)?;

        let mut cbv_heap = DescriptorHeap::constant_buffer_view_heap(&device, FRAME_COUNT)?;

        let constant_buffer = [
            glam::Mat4::from_translation(Vec3::new(0.0, -1.0, 1.5))
                * glam::Mat4::from_rotation_y(PI),
            glam::Mat4::perspective_lh(PI / 2.0, aspect_ratio, 0.1, 100.0),
        ];
        let constant_buffer_size = align_data(
            std::mem::size_of_val(&constant_buffer),
            D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT as usize,
        );
        let constant_buffers: [MappedBuffer; FRAME_COUNT as usize] =
            array_init::try_array_init(|_| {
                let buffer = create_constant_buffer(&device, constant_buffer_size)?;

                //let matrix = glam::Mat4::IDENTITY;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        std::ptr::addr_of!(constant_buffer),
                        buffer.data as _,
                        1,
                    )
                };

                unsafe {
                    device.CreateConstantBufferView(
                        &D3D12_CONSTANT_BUFFER_VIEW_DESC {
                            BufferLocation: buffer.buffer.GetGPUVirtualAddress(),
                            SizeInBytes: buffer.size as u32,
                        },
                        cbv_heap.allocate_handle()?,
                    )
                };

                Ok(buffer)
            })?;

        let mut fence_values = [0; 2];

        let fence = unsafe {
            device.CreateFence(fence_values[frame_index as usize], D3D12_FENCE_FLAG_NONE)
        }?;

        fence_values[frame_index as usize] += 1;

        let fence_event = unsafe { CreateEventA(std::ptr::null(), false, false, None) }?;

        let resources = RendererResources {
            hwnd,
            dxgi_factory,
            device,

            command_queue,
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
            fence,
            fence_values,
            fence_event,

            constant_buffers,
            depth_buffers,
        };

        let mut renderer = Renderer {
            resources: Some(resources),
        };

        renderer.wait_for_gpu()?;

        Ok(renderer)
    }

    fn populate_command_list(&self) -> Result<()> {
        ensure!(self.resources.is_some());
        let resources = self.resources.as_ref().unwrap();

        let command_allocator = &resources.command_allocators[resources.frame_index as usize];
        unsafe {
            command_allocator.Reset()?;
        }

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

    pub fn wait_for_gpu(&mut self) -> Result<()> {
        ensure!(self.resources.is_some());
        let resources = self.resources.as_mut().unwrap();
        let fence = &resources.fence;
        let frame_index = resources.frame_index as usize;
        let fence_value = &mut resources.fence_values[frame_index];

        unsafe {
            resources.command_queue.Signal(fence, *fence_value)?;

            resources
                .fence
                .SetEventOnCompletion(*fence_value, resources.fence_event)?;

            WaitForSingleObject(resources.fence_event, INFINITE);
        }

        *fence_value += 1;

        Ok(())
    }

    fn move_to_next_frame(&mut self) -> Result<()> {
        ensure!(self.resources.is_some());
        let resources = self.resources.as_mut().unwrap();

        let current_fence_value = resources.fence_values[resources.frame_index as usize];

        unsafe {
            resources
                .command_queue
                .Signal(&resources.fence, current_fence_value)
        }?;

        resources.frame_index = unsafe { resources.swap_chain.GetCurrentBackBufferIndex() };

        let completed_value = unsafe { resources.fence.GetCompletedValue() };
        if completed_value < resources.fence_values[resources.frame_index as usize] {
            unsafe {
                resources.fence.SetEventOnCompletion(
                    resources.fence_values[resources.frame_index as usize],
                    resources.fence_event,
                )?;
                WaitForSingleObject(resources.fence_event, INFINITE);
            }
        }
        resources.fence_values[resources.frame_index as usize] = current_fence_value + 1;

        Ok(())
    }

    pub fn render(&mut self) -> Result<()> {
        ensure!(self.resources.is_some());
        let resources = self.resources.as_ref().unwrap();

        self.populate_command_list()?;

        let command_list = ID3D12CommandList::from(&resources.command_list);
        unsafe {
            resources
                .command_queue
                .ExecuteCommandLists(&[Some(command_list)])
        };

        unsafe { resources.swap_chain.Present(1, 0) }.ok()?;

        self.move_to_next_frame()?;

        Ok(())
    }
}
