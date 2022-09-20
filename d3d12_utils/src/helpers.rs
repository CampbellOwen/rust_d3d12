use anyhow::{Context, Result};

use hassle_rs::{compile_hlsl, validate_dxil};
use windows::Win32::Graphics::{
    Direct3D::*,
    Direct3D12::*,
    Dxgi::{Common::*, *},
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

pub fn create_descriptor_table(
    shader_visiblity: D3D12_SHADER_VISIBILITY,
    descriptor_ranges: &[D3D12_DESCRIPTOR_RANGE],
) -> D3D12_ROOT_PARAMETER {
    D3D12_ROOT_PARAMETER {
        ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
        ShaderVisibility: shader_visiblity,
        Anonymous: D3D12_ROOT_PARAMETER_0 {
            DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                NumDescriptorRanges: descriptor_ranges.len() as u32,
                pDescriptorRanges: descriptor_ranges.as_ptr(),
            },
        },
    }
}

pub fn create_root_signature(device: &ID3D12Device4) -> Result<ID3D12RootSignature> {
    let root_parameters = [
        create_descriptor_table(
            D3D12_SHADER_VISIBILITY_ALL,
            &[D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_CBV,
                NumDescriptors: 1,
                BaseShaderRegister: 0,
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
            }],
        ),
        create_descriptor_table(
            D3D12_SHADER_VISIBILITY_PIXEL,
            &[D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: 1,
                BaseShaderRegister: 0,
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
            }],
        ),
    ];

    let static_samplers = [D3D12_STATIC_SAMPLER_DESC {
        Filter: D3D12_FILTER_MIN_MAG_MIP_POINT,
        AddressU: D3D12_TEXTURE_ADDRESS_MODE_BORDER,
        AddressV: D3D12_TEXTURE_ADDRESS_MODE_BORDER,
        AddressW: D3D12_TEXTURE_ADDRESS_MODE_BORDER,
        MipLODBias: 0.0f32,
        MaxAnisotropy: 0,
        ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
        BorderColor: D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK,
        MinLOD: 0.0f32,
        MaxLOD: D3D12_FLOAT32_MAX,
        ShaderRegister: 0,
        RegisterSpace: 0,
        ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
    }];

    let desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: root_parameters.len() as u32,
        pParameters: root_parameters.as_ptr(),
        Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        pStaticSamplers: static_samplers.as_ptr(),
        NumStaticSamplers: static_samplers.len() as u32,
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
    input_element_descs: &[D3D12_INPUT_ELEMENT_DESC],
    vertex_shader: &CompiledShader,
    pixel_shader: &CompiledShader,
    num_render_targets: u32,
) -> Result<ID3D12PipelineState> {
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
            CullMode: D3D12_CULL_MODE_BACK,
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
        NumRenderTargets: num_render_targets,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    for i in 0..num_render_targets as usize {
        desc.RTVFormats[i] = DXGI_FORMAT_R8G8B8A8_UNORM;
    }

    let pso = unsafe { device.CreateGraphicsPipelineState(&desc) }?;

    Ok(pso)
}

pub fn align_data(location: usize, alignment: usize) -> usize {
    if alignment == 0 || (alignment & (alignment - 1) != 0) {
        panic!("Non power of 2 alignment");
    }

    (location + (alignment - 1)) & !(alignment - 1)
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
