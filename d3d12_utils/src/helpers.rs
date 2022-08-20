use std::ffi::c_void;

use anyhow::{Context, Result};

use hassle_rs::{compile_hlsl, validate_dxil};
use windows::{
    core::PCSTR,
    Win32::Graphics::{
        Direct3D::*,
        Direct3D12::*,
        Dxgi::{Common::*, *},
    },
};

pub fn get_hardware_adapter(
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

pub fn create_dxgi_factory() -> Result<IDXGIFactory5> {
    let dxgi_factory_flags = if cfg!(debug_assertions) {
        DXGI_CREATE_FACTORY_DEBUG
    } else {
        0
    };

    let factory = unsafe { CreateDXGIFactory2(dxgi_factory_flags) }?;

    Ok(factory)
}

pub fn create_device(
    adapter: &IDXGIAdapter1,
    feature_level: D3D_FEATURE_LEVEL,
) -> Result<ID3D12Device4> {
    let mut device: Option<ID3D12Device4> = None;
    unsafe { D3D12CreateDevice(adapter, feature_level, &mut device) }?;
    Ok(device.unwrap())
}

pub fn create_root_signature(device: &ID3D12Device4) -> Result<ID3D12RootSignature> {
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

pub fn create_pipeline_state(
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
        DSVFormat: DXGI_FORMAT_D32_FLOAT,
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

pub fn create_vertex_buffer<T: Sized + std::fmt::Debug>(
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
        StrideInBytes: std::mem::size_of::<T>() as u32,
        SizeInBytes: std::mem::size_of_val(vertices) as u32,
    };

    Ok((vertex_buffer, vbv))
}

pub fn create_index_buffer(
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

pub fn align_data(location: usize, alignment: usize) -> usize {
    if alignment == 0 || (alignment & (alignment - 1) != 0) {
        panic!("Non power of 2 alignment");
    }

    (location + (alignment - 1)) & !(alignment - 1)
}

#[derive(Debug)]
pub struct MappedBuffer {
    pub buffer: ID3D12Resource,
    pub size: usize,
    pub data: *mut c_void,
}

#[derive(Debug)]
pub struct Tex2D {
    pub resource: ID3D12Resource,
    pub width: usize,
    pub height: usize,
}

pub fn create_depth_stencil_buffer(
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

pub fn create_constant_buffer(device: &ID3D12Device4, size: usize) -> Result<MappedBuffer> {
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

pub fn transition_barrier(
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
