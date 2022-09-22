use crate::{
    CommandQueue, Descriptor, DescriptorManager, DescriptorType, Heap, Resource, UploadRingBuffer,
};
use anyhow::{ensure, Context, Result};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

const DEFAULT_TEXTURE_HEAP_SIZE: usize = 2160 * 3840 * 4 * 100;

#[derive(Debug, Clone, Copy)]
pub enum TextureDimension {
    One(usize),
    Two(usize, u32),
    Three(usize, u32, u16),
}
#[derive(Debug)]
pub struct Texture {
    pub texture_type: TextureDimension,
    pub format: DXGI_FORMAT,
    pub array_size: u16,
    pub num_mips: u16,
    pub resource: Resource,
}

#[derive(Debug)]
pub struct TextureManager {
    texture_heap: Heap,
    rtv_descriptors: Vec<Descriptor>,
    srv_descriptors: Vec<Descriptor>,
    uav_descriptors: Vec<Descriptor>,
    textures: Vec<Texture>,
}

#[derive(Debug, Clone)]
pub struct TextureHandle {
    index: usize,
    rtv_index: Option<usize>,
    srv_index: Option<usize>,
    uav_index: Option<usize>,
}

const MAX_NUM_SUBRESOURCES: usize = 10;
impl TextureManager {
    pub fn new(device: &ID3D12Device4, heap_size: Option<usize>) -> Result<Self> {
        let heap_size = if let Some(heap_size) = heap_size {
            heap_size
        } else {
            DEFAULT_TEXTURE_HEAP_SIZE
        };

        let heap = Heap::create_default_heap(device, heap_size, "Texture Manager Heap")?;

        Ok(TextureManager {
            texture_heap: heap,
            rtv_descriptors: Vec::new(),
            srv_descriptors: Vec::new(),
            uav_descriptors: Vec::new(),
            textures: Vec::new(),
        })
    }

    pub fn create_texture(
        &mut self,
        device: &ID3D12Device4,
        uploader: &mut UploadRingBuffer,
        dependent_queue: Option<&CommandQueue>,
        texture_type: TextureDimension,
        format: DXGI_FORMAT,
        array_size: u16,
        num_mips: u16,
        data: &[u8],
    ) -> Result<TextureHandle> {
        let (dimension, width, height, depth) = match texture_type {
            TextureDimension::One(width) => (D3D12_RESOURCE_DIMENSION_TEXTURE1D, width, 1, 1),
            TextureDimension::Two(width, height) => (
                D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                width,
                height,
                array_size,
            ),
            TextureDimension::Three(width, height, depth) => {
                (D3D12_RESOURCE_DIMENSION_TEXTURE3D, width, height, depth)
            }
        };

        let num_subresources = depth * num_mips;

        ensure!(num_subresources as usize <= MAX_NUM_SUBRESOURCES);

        let texture_desc = D3D12_RESOURCE_DESC {
            Dimension: dimension,
            Width: width as u64,
            Height: height as u32,
            DepthOrArraySize: depth as u16,
            MipLevels: num_mips as u16,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            ..Default::default()
        };

        let mut layouts: [D3D12_PLACED_SUBRESOURCE_FOOTPRINT; MAX_NUM_SUBRESOURCES] =
            Default::default();
        let mut num_rows: [u32; MAX_NUM_SUBRESOURCES] = Default::default();
        let mut row_size_bytes: [u64; MAX_NUM_SUBRESOURCES] = Default::default();
        let mut total_bytes = 0;

        unsafe {
            device.GetCopyableFootprints(
                &texture_desc,
                0,
                num_subresources as u32,
                0,
                layouts.as_mut_ptr(),
                num_rows.as_mut_ptr(),
                row_size_bytes.as_mut_ptr(),
                &mut total_bytes,
            );
        }

        let texture_resource = self.texture_heap.create_resource(
            device,
            &texture_desc,
            D3D12_RESOURCE_STATE_COMMON,
            false,
        )?;

        let upload_context = uploader.allocate(total_bytes as usize)?;

        let mut data_offset = 0;
        for array_index in 0..array_size {
            for mip_index in 0..num_mips {
                let layout_index = (mip_index + (array_index * num_mips)) as usize;
                let layout = &layouts[layout_index];
                let row_bytes = row_size_bytes[layout_index];
                let mut resource_offset = layout.Offset;

                for _ in 0..layout.Footprint.Depth {
                    for _ in 0..layout.Footprint.Height {
                        let row = &data[data_offset as usize..(data_offset + row_bytes) as usize];

                        upload_context
                            .sub_resource
                            .copy_to_offset_from(resource_offset as usize, row)?;

                        data_offset += row_bytes;
                        resource_offset += layout.Footprint.RowPitch as u64;
                    }
                }
            }
        }

        for subresource_index in 0..num_subresources {
            let mut layout = layouts[subresource_index as usize];
            layout.Offset += upload_context.sub_resource.offset as u64;

            let from = D3D12_TEXTURE_COPY_LOCATION {
                pResource: Some(upload_context.sub_resource.resource.device_resource.clone()),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: layout,
                },
            };
            let to = D3D12_TEXTURE_COPY_LOCATION {
                pResource: Some(texture_resource.device_resource.clone()),
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    SubresourceIndex: subresource_index as u32,
                },
            };

            unsafe {
                upload_context.command_list.CopyTextureRegion(
                    &to,
                    0,
                    0,
                    00,
                    &from,
                    std::ptr::null(),
                );
            }
        }

        upload_context.submit(dependent_queue)?;

        let texture = Texture {
            texture_type,
            format,
            num_mips,
            array_size,
            resource: texture_resource,
        };

        self.textures.push(texture);
        let texture_index = self.textures.len() - 1;

        Ok(TextureHandle {
            index: texture_index,
            rtv_index: None,
            srv_index: None,
            uav_index: None,
        })
    }

    pub fn get_texture(&self, handle: &TextureHandle) -> Result<&Texture> {
        self.textures
            .get(handle.index)
            .context("Invalid texture handle")
    }

    pub fn get_rtv(
        &mut self,
        device: &ID3D12Device4,
        descriptor_manager: &mut DescriptorManager,
        handle: &mut TextureHandle,
    ) -> Result<Descriptor> {
        if let Some(idx) = handle.rtv_index {
            return self
                .rtv_descriptors
                .get(idx)
                .copied()
                .context("Invalid rtv index");
        }

        let descriptor = descriptor_manager.allocate(DescriptorType::RenderTargetView)?;
        self.rtv_descriptors.push(descriptor);
        handle.rtv_index = Some(self.rtv_descriptors.len() - 1);

        Ok(descriptor)
    }

    pub fn get_srv(
        &mut self,
        device: &ID3D12Device4,
        descriptor_manager: &mut DescriptorManager,
        handle: &mut TextureHandle,
    ) -> Result<Descriptor> {
        if let Some(idx) = handle.srv_index {
            return self
                .srv_descriptors
                .get(idx)
                .copied()
                .context("Invalid rtv index");
        }

        let descriptor = descriptor_manager.allocate(DescriptorType::Resource)?;

        let texture = &self.textures[handle.index];
        let (view_dimension, anonymous_member) = match texture.texture_type {
            TextureDimension::One(_) => {
                if texture.array_size > 1 {
                    (
                        D3D12_SRV_DIMENSION_TEXTURE1DARRAY,
                        D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                            Texture1DArray: D3D12_TEX1D_ARRAY_SRV {
                                MostDetailedMip: 0,
                                MipLevels: texture.num_mips as u32,
                                FirstArraySlice: 0,
                                ArraySize: texture.array_size as u32,
                                ResourceMinLODClamp: 0.0,
                            },
                        },
                    )
                } else {
                    (
                        D3D12_SRV_DIMENSION_TEXTURE1D,
                        D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                            Texture1D: D3D12_TEX1D_SRV {
                                MostDetailedMip: 0,
                                MipLevels: texture.num_mips as u32,
                                ResourceMinLODClamp: 0.0,
                            },
                        },
                    )
                }
            }
            TextureDimension::Two(_, _) => {
                if texture.array_size > 1 {
                    (
                        D3D12_SRV_DIMENSION_TEXTURE2DARRAY,
                        D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                            Texture2DArray: D3D12_TEX2D_ARRAY_SRV {
                                MostDetailedMip: 0,
                                MipLevels: texture.num_mips as u32,
                                FirstArraySlice: 0,
                                ArraySize: texture.array_size as u32,
                                PlaneSlice: 0,
                                ResourceMinLODClamp: 0.0,
                            },
                        },
                    )
                } else {
                    (
                        D3D12_SRV_DIMENSION_TEXTURE2D,
                        D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                            Texture2D: D3D12_TEX2D_SRV {
                                MostDetailedMip: 0,
                                MipLevels: texture.num_mips as u32,
                                PlaneSlice: 0,
                                ResourceMinLODClamp: 0.0,
                            },
                        },
                    )
                }
            }
            TextureDimension::Three(_, _, _) => (
                D3D12_SRV_DIMENSION_TEXTURE3D,
                D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture3D: D3D12_TEX3D_SRV {
                        MostDetailedMip: 0,
                        MipLevels: texture.num_mips as u32,
                        ResourceMinLODClamp: 0.0,
                    },
                },
            ),
        };

        unsafe {
            device.CreateShaderResourceView(
                &texture.resource.device_resource,
                &D3D12_SHADER_RESOURCE_VIEW_DESC {
                    Format: texture.format,
                    ViewDimension: view_dimension,
                    Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                    Anonymous: anonymous_member,
                },
                descriptor_manager.get_cpu_handle(&descriptor)?,
            );
        }

        self.srv_descriptors.push(descriptor);
        handle.srv_index = Some(self.srv_descriptors.len() - 1);

        Ok(descriptor)
    }
}
