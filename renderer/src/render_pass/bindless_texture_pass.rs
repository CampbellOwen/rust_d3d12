use anyhow::{Context, Result};
use d3d12_utils::{
    align_data, compile_pixel_shader, compile_vertex_shader, create_pipeline_state,
    create_root_signature, DescriptorHandle, DescriptorType, Resource, TextureHandle,
};
use windows::{
    core::PCSTR,
    Win32::Graphics::{
        Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, Direct3D12::*, Dxgi::Common::*,
    },
};

use crate::{
    object::Object,
    renderer::{Camera, Resources},
};

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
pub struct BindlessTexturePass<const FRAME_COUNT: usize> {
    #[allow(dead_code)]
    camera_constant_buffers: [Resource; FRAME_COUNT],
    camera_cbv_descriptors: [DescriptorHandle; FRAME_COUNT],
    #[allow(dead_code)]
    material_constant_buffers: [Resource; FRAME_COUNT],
    material_descriptors: [DescriptorHandle; FRAME_COUNT],
    #[allow(dead_code)]
    model_constant_buffers: [Resource; FRAME_COUNT],
    model_descriptors: [DescriptorHandle; FRAME_COUNT],

    root_signature: ID3D12RootSignature,
    pso: ID3D12PipelineState,
}

impl<const FRAME_COUNT: usize> BindlessTexturePass<FRAME_COUNT> {
    pub fn new(resources: &mut Resources) -> Result<Self> {
        let root_signature = create_root_signature(&resources.device)?;

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
            &resources.device,
            &root_signature,
            &input_element_descs,
            &vertex_shader,
            &pixel_shader,
            1,
        )?;

        let camera_buffer_size = align_data(
            std::mem::size_of::<Camera>(),
            D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT as usize,
        );

        let mut camera_cbv_descriptors: [DescriptorHandle; FRAME_COUNT] =
            array_init::array_init(|_| DescriptorHandle::default());
        let camera_constant_buffers: [Resource; FRAME_COUNT] =
            array_init::try_array_init(|i| -> Result<Resource> {
                let buffer = Resource::create_committed(
                    &resources.device,
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

                buffer.copy_from(&[resources.camera])?;

                let cbv_descriptor = resources
                    .descriptor_manager
                    .allocate(DescriptorType::Resource)?;
                camera_cbv_descriptors[i] = cbv_descriptor;

                unsafe {
                    resources.device.CreateConstantBufferView(
                        &D3D12_CONSTANT_BUFFER_VIEW_DESC {
                            BufferLocation: buffer.gpu_address(),
                            SizeInBytes: buffer.size as u32,
                        },
                        resources
                            .descriptor_manager
                            .get_cpu_handle(&cbv_descriptor)?,
                    )
                };

                Ok(buffer)
            })?;

        let material_buffer_size = align_data(
            std::mem::size_of::<MaterialConstantBuffer>(),
            D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT as usize,
        );
        let mut material_descriptors: [DescriptorHandle; FRAME_COUNT] =
            array_init::array_init(|_| DescriptorHandle::default());
        let material_constant_buffers: [Resource; FRAME_COUNT] =
            array_init::try_array_init(|i| -> Result<Resource> {
                let buffer = Resource::create_committed(
                    &resources.device,
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

                let cbv_descriptor = resources
                    .descriptor_manager
                    .allocate(DescriptorType::Resource)?;
                material_descriptors[i] = cbv_descriptor;

                unsafe {
                    resources.device.CreateConstantBufferView(
                        &D3D12_CONSTANT_BUFFER_VIEW_DESC {
                            BufferLocation: buffer.gpu_address(),
                            SizeInBytes: buffer.size as u32,
                        },
                        resources
                            .descriptor_manager
                            .get_cpu_handle(&cbv_descriptor)?,
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
        let mut model_descriptors: [DescriptorHandle; FRAME_COUNT] =
            array_init::array_init(|_| DescriptorHandle::default());
        let model_constant_buffers: [Resource; FRAME_COUNT] =
            array_init::try_array_init(|i| -> Result<Resource> {
                let buffer = Resource::create_committed(
                    &resources.device,
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

                let cbv_descriptor = resources
                    .descriptor_manager
                    .allocate(DescriptorType::Resource)?;
                model_descriptors[i] = cbv_descriptor;

                unsafe {
                    resources.device.CreateConstantBufferView(
                        &D3D12_CONSTANT_BUFFER_VIEW_DESC {
                            BufferLocation: buffer.gpu_address(),
                            SizeInBytes: buffer.size as u32,
                        },
                        resources
                            .descriptor_manager
                            .get_cpu_handle(&cbv_descriptor)?,
                    )
                };

                Ok(buffer)
            })?;

        Ok(BindlessTexturePass {
            camera_constant_buffers,
            camera_cbv_descriptors,
            material_constant_buffers,
            material_descriptors,
            model_constant_buffers,
            model_descriptors,
            root_signature,
            pso,
        })
    }
}

impl<const FRAME_COUNT: usize> BindlessTexturePass<FRAME_COUNT> {
    pub fn render(
        &mut self,
        command_list: &ID3D12GraphicsCommandList,
        resources: &mut Resources,
        render_target_handle: &TextureHandle,
        depth_buffer_handle: &TextureHandle,
        objects: &[Object],
    ) -> Result<()> {
        unsafe {
            command_list.SetPipelineState(&self.pso);
        }
        let camera_cb_handle = resources
            .descriptor_manager
            .get_gpu_handle(&self.camera_cbv_descriptors[resources.frame_index as usize])?;

        let model_cb_handle = resources
            .descriptor_manager
            .get_gpu_handle(&self.model_descriptors[resources.frame_index as usize])?;

        let material_cb_handle = resources
            .descriptor_manager
            .get_gpu_handle(&self.material_descriptors[resources.frame_index as usize])?;

        let camera_cb = &self.camera_constant_buffers[resources.frame_index as usize];
        camera_cb.copy_from(&[resources.camera])?;

        unsafe {
            command_list.SetDescriptorHeaps(&[Some(
                resources
                    .descriptor_manager
                    .get_heap(DescriptorType::Resource)?,
            )]);
            command_list.SetGraphicsRootSignature(&self.root_signature);

            command_list.SetGraphicsRootDescriptorTable(0, camera_cb_handle);
            command_list.SetGraphicsRootDescriptorTable(1, material_cb_handle);
            command_list.SetGraphicsRootDescriptorTable(2, model_cb_handle);

            command_list.RSSetViewports(&[resources.viewport]);
            command_list.RSSetScissorRects(&[resources.scissor_rect]);
        }

        let rtv_handle = resources.texture_manager.get_rtv(render_target_handle)?;
        let rtv = resources.descriptor_manager.get_cpu_handle(&rtv_handle)?;

        let dsv_handle = resources.texture_manager.get_dsv(depth_buffer_handle)?;
        let dsv = resources.descriptor_manager.get_cpu_handle(&dsv_handle)?;

        unsafe {
            command_list.OMSetRenderTargets(1, &rtv, false, &dsv);
            command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        }

        for object in objects {
            let material_cb = &self.material_constant_buffers[resources.frame_index as usize];
            material_cb.copy_from(&[MaterialConstantBuffer {
                texture_index: object.texture.srv_index.context("Need srv")? as u32,
            }])?;

            let model_cb = &self.model_constant_buffers[resources.frame_index as usize];
            model_cb.copy_from(&[ModelConstantBuffer {
                M: glam::Mat4::from_translation(object.position)
                    * glam::Mat4::from_rotation_y(std::f32::consts::PI * -0.9),
            }])?;

            let vbv = object.mesh.vbv.context("Object vertex buffer view")?;
            let ibv = object.mesh.ibv.context("Object index buffer view")?;

            unsafe {
                command_list.IASetVertexBuffers(0, &[vbv]);
                command_list.IASetIndexBuffer(&ibv);
                command_list.DrawIndexedInstanced(object.mesh.num_vertices as u32, 1, 0, 0, 0);
            }
        }

        Ok(())
    }
}
