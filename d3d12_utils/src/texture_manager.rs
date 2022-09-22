use crate::{
    CommandQueue, DescriptorHandle, DescriptorManager, DescriptorType, Heap, Resource,
    UploadRingBuffer,
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

#[derive(Debug, Clone, Copy)]
pub struct TextureInfo {
    pub dimension: TextureDimension,
    pub format: DXGI_FORMAT,
    pub array_size: u16,
    pub num_mips: u16,
    pub is_render_target: bool,
    pub is_depth_buffer: bool,
    pub is_unordered_access: bool,
}

#[derive(Debug)]
pub struct Texture {
    pub info: TextureInfo,
    pub resource: Resource,
}

#[derive(Debug)]
pub struct TextureManager {
    texture_heap: Heap,
    rtv_descriptors: Vec<DescriptorHandle>,
    srv_descriptors: Vec<DescriptorHandle>,
    uav_descriptors: Vec<DescriptorHandle>,
    dsv_descriptors: Vec<DescriptorHandle>,
    textures: Vec<Texture>,
}

#[derive(Debug, Default, Clone)]
pub struct TextureHandle {
    index: usize,
    rtv_index: Option<usize>,
    srv_index: Option<usize>,
    uav_index: Option<usize>,
    dsv_index: Option<usize>,
}

const MAX_NUM_SUBRESOURCES: usize = 32;
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
            dsv_descriptors: Vec::new(),
            textures: Vec::new(),
        })
    }

    pub fn add_texture(
        &mut self,
        device: &ID3D12Device4,
        descriptor_manager: &mut DescriptorManager,
        texture: Texture,
    ) -> Result<TextureHandle> {
        let texture_info = &texture.info;

        let rtv_index = if texture_info.is_render_target {
            let rtv_handle = self.create_rtv(device, descriptor_manager, &texture)?;
            self.rtv_descriptors.push(rtv_handle);
            Some(self.rtv_descriptors.len() - 1)
        } else {
            None
        };

        let srv_index = if !texture_info.is_depth_buffer {
            let srv_handle = self.create_srv(device, descriptor_manager, &texture)?;
            self.srv_descriptors.push(srv_handle);
            Some(self.srv_descriptors.len() - 1)
        } else {
            None
        };

        let uav_index = if texture_info.is_unordered_access {
            let uav_handle = self.create_uav(device, descriptor_manager, &texture)?;
            self.uav_descriptors.push(uav_handle);
            Some(self.uav_descriptors.len() - 1)
        } else {
            None
        };

        let dsv_index = if texture_info.is_depth_buffer {
            let dsv_handle = self.create_dsv(device, descriptor_manager, &texture)?;
            self.dsv_descriptors.push(dsv_handle);
            Some(self.dsv_descriptors.len() - 1)
        } else {
            None
        };

        self.textures.push(texture);
        let index = self.textures.len() - 1;

        Ok(TextureHandle {
            index,
            rtv_index,
            srv_index,
            uav_index,
            dsv_index,
        })
    }

    pub fn create_empty_texture(
        &mut self,
        device: &ID3D12Device4,
        texture_info: TextureInfo,
        descriptor_manager: &mut DescriptorManager,
    ) -> Result<TextureHandle> {
        let (dimension, width, height, depth) = match texture_info.dimension {
            TextureDimension::One(width) => (D3D12_RESOURCE_DIMENSION_TEXTURE1D, width, 1, 1),
            TextureDimension::Two(width, height) => (
                D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                width,
                height,
                texture_info.array_size,
            ),
            TextureDimension::Three(width, height, depth) => {
                (D3D12_RESOURCE_DIMENSION_TEXTURE3D, width, height, depth)
            }
        };

        let num_subresources = depth * texture_info.num_mips;

        ensure!(num_subresources as usize <= MAX_NUM_SUBRESOURCES);

        let mut flags: u32 = 0;
        if texture_info.is_depth_buffer {
            flags |= D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL.0;
        }
        if texture_info.is_render_target {
            flags |= D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET.0;
        }
        if texture_info.is_unordered_access {
            flags |= D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS.0;
        }

        let texture_desc = D3D12_RESOURCE_DESC {
            Dimension: dimension,
            Width: width as u64,
            Height: height as u32,
            DepthOrArraySize: depth as u16,
            MipLevels: texture_info.num_mips as u16,
            Format: texture_info.format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAGS(flags),
            ..Default::default()
        };

        let texture_resource = self.texture_heap.create_resource(
            device,
            &texture_desc,
            D3D12_RESOURCE_STATE_COMMON,
            false,
        )?;
        let texture = Texture {
            info: texture_info,
            resource: texture_resource,
        };

        let rtv_index = if texture_info.is_render_target {
            let rtv_handle = self.create_rtv(device, descriptor_manager, &texture)?;
            self.rtv_descriptors.push(rtv_handle);
            Some(self.rtv_descriptors.len() - 1)
        } else {
            None
        };

        let srv_index = if !texture_info.is_depth_buffer {
            let srv_handle = self.create_srv(device, descriptor_manager, &texture)?;
            self.srv_descriptors.push(srv_handle);
            Some(self.srv_descriptors.len() - 1)
        } else {
            None
        };

        let uav_index = if texture_info.is_unordered_access {
            let uav_handle = self.create_uav(device, descriptor_manager, &texture)?;
            self.uav_descriptors.push(uav_handle);
            Some(self.uav_descriptors.len() - 1)
        } else {
            None
        };

        let dsv_index = if texture_info.is_depth_buffer {
            let dsv_handle = self.create_dsv(device, descriptor_manager, &texture)?;
            self.dsv_descriptors.push(dsv_handle);
            Some(self.dsv_descriptors.len() - 1)
        } else {
            None
        };

        self.textures.push(texture);
        let texture_index = self.textures.len() - 1;

        Ok(TextureHandle {
            index: texture_index,
            rtv_index,
            srv_index,
            uav_index,
            dsv_index,
        })
    }

    pub fn create_texture(
        &mut self,
        device: &ID3D12Device4,
        uploader: &mut UploadRingBuffer,
        dependent_queue: Option<&CommandQueue>,
        descriptor_manager: &mut DescriptorManager,
        texture_info: TextureInfo,
        data: &[u8],
    ) -> Result<TextureHandle> {
        let texture_handle = self.create_empty_texture(device, texture_info, descriptor_manager)?;
        let texture = self.get_texture(&texture_handle)?;

        let (dimension, width, height, depth) = match texture_info.dimension {
            TextureDimension::One(width) => (D3D12_RESOURCE_DIMENSION_TEXTURE1D, width, 1, 1),
            TextureDimension::Two(width, height) => (
                D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                width,
                height,
                texture_info.array_size,
            ),
            TextureDimension::Three(width, height, depth) => {
                (D3D12_RESOURCE_DIMENSION_TEXTURE3D, width, height, depth)
            }
        };

        let num_subresources = depth * texture_info.num_mips;

        let texture_desc = D3D12_RESOURCE_DESC {
            Dimension: dimension,
            Width: width as u64,
            Height: height as u32,
            DepthOrArraySize: depth as u16,
            MipLevels: texture_info.num_mips as u16,
            Format: texture_info.format,
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

        let upload_context = uploader.allocate(total_bytes as usize)?;

        let mut data_offset = 0;
        for array_index in 0..texture_info.array_size {
            for mip_index in 0..texture_info.num_mips {
                let layout_index = (mip_index + (array_index * texture_info.num_mips)) as usize;
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
                pResource: Some(texture.resource.device_resource.clone()),
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

        Ok(texture_handle)
    }

    pub fn get_texture(&self, handle: &TextureHandle) -> Result<&Texture> {
        self.textures
            .get(handle.index)
            .context("Invalid texture handle")
    }

    pub fn get_rtv(&self, handle: &TextureHandle) -> Result<DescriptorHandle> {
        let rtv_index = handle.rtv_index.context("No rtv for texture")?;
        self.rtv_descriptors
            .get(rtv_index)
            .copied()
            .context("Invalid rtv index")
    }

    pub fn get_dsv(&self, handle: &TextureHandle) -> Result<DescriptorHandle> {
        let dsv_index = handle.dsv_index.context("No dsv for texture")?;
        self.dsv_descriptors
            .get(dsv_index)
            .copied()
            .context("Invalid dsv index")
    }
    pub fn get_uav(&self, handle: &TextureHandle) -> Result<DescriptorHandle> {
        let uav_index = handle.uav_index.context("No uav for texture")?;
        self.uav_descriptors
            .get(uav_index)
            .copied()
            .context("Invalid uav index")
    }

    fn create_uav(
        &mut self,
        device: &ID3D12Device4,
        descriptor_manager: &mut DescriptorManager,
        texture: &Texture,
    ) -> Result<DescriptorHandle> {
        let descriptor = descriptor_manager.allocate(DescriptorType::Resource)?;

        let (view_dimension, anonymous_member) = match texture.info.dimension {
            TextureDimension::One(_) => {
                if texture.info.array_size > 1 {
                    (
                        D3D12_UAV_DIMENSION_TEXTURE1DARRAY,
                        D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                            Texture1DArray: D3D12_TEX1D_ARRAY_UAV {
                                FirstArraySlice: 0,
                                ArraySize: texture.info.array_size as u32,
                                MipSlice: 0,
                            },
                        },
                    )
                } else {
                    (
                        D3D12_UAV_DIMENSION_TEXTURE1D,
                        D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                            Texture1D: D3D12_TEX1D_UAV { MipSlice: 0 },
                        },
                    )
                }
            }
            TextureDimension::Two(_, _) => {
                if texture.info.array_size > 1 {
                    (
                        D3D12_UAV_DIMENSION_TEXTURE2DARRAY,
                        D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                            Texture2DArray: D3D12_TEX2D_ARRAY_UAV {
                                FirstArraySlice: 0,
                                ArraySize: texture.info.array_size as u32,
                                PlaneSlice: 0,
                                MipSlice: 0,
                            },
                        },
                    )
                } else {
                    (
                        D3D12_UAV_DIMENSION_TEXTURE2D,
                        D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                            Texture2D: D3D12_TEX2D_UAV {
                                PlaneSlice: 0,
                                MipSlice: 0,
                            },
                        },
                    )
                }
            }
            TextureDimension::Three(_, _, _) => (
                D3D12_UAV_DIMENSION_TEXTURE3D,
                D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
                    Texture3D: D3D12_TEX3D_UAV {
                        MipSlice: 0,
                        FirstWSlice: 0,
                        WSize: u32::MAX,
                    },
                },
            ),
        };

        unsafe {
            device.CreateUnorderedAccessView(
                &texture.resource.device_resource,
                None,
                &D3D12_UNORDERED_ACCESS_VIEW_DESC {
                    Format: texture.info.format,
                    ViewDimension: view_dimension,
                    Anonymous: anonymous_member,
                },
                descriptor_manager.get_cpu_handle(&descriptor)?,
            );
        }

        Ok(descriptor)
    }

    fn create_dsv(
        &mut self,
        device: &ID3D12Device4,
        descriptor_manager: &mut DescriptorManager,
        texture: &Texture,
    ) -> Result<DescriptorHandle> {
        let descriptor = descriptor_manager.allocate(DescriptorType::DepthStencilView)?;

        let (view_dimension, anonymous_member) = match texture.info.dimension {
            TextureDimension::One(_) => {
                if texture.info.array_size > 1 {
                    Ok((
                        D3D12_DSV_DIMENSION_TEXTURE1DARRAY,
                        D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
                            Texture1DArray: D3D12_TEX1D_ARRAY_DSV {
                                FirstArraySlice: 0,
                                ArraySize: texture.info.array_size as u32,
                                MipSlice: 0,
                            },
                        },
                    ))
                } else {
                    Ok((
                        D3D12_DSV_DIMENSION_TEXTURE1D,
                        D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
                            Texture1D: D3D12_TEX1D_DSV { MipSlice: 0 },
                        },
                    ))
                }
            }
            TextureDimension::Two(_, _) => {
                if texture.info.array_size > 1 {
                    Ok((
                        D3D12_DSV_DIMENSION_TEXTURE2DARRAY,
                        D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
                            Texture2DArray: D3D12_TEX2D_ARRAY_DSV {
                                FirstArraySlice: 0,
                                ArraySize: texture.info.array_size as u32,
                                MipSlice: 0,
                            },
                        },
                    ))
                } else {
                    Ok((
                        D3D12_DSV_DIMENSION_TEXTURE2D,
                        D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
                            Texture2D: D3D12_TEX2D_DSV { MipSlice: 0 },
                        },
                    ))
                }
            }
            TextureDimension::Three(_, _, _) => (None.context("Cannot have a 3D depth buffer")),
        }?;

        unsafe {
            device.CreateDepthStencilView(
                &texture.resource.device_resource,
                &D3D12_DEPTH_STENCIL_VIEW_DESC {
                    Format: texture.info.format,
                    ViewDimension: view_dimension,
                    Anonymous: anonymous_member,
                    Flags: D3D12_DSV_FLAG_NONE,
                },
                descriptor_manager.get_cpu_handle(&descriptor)?,
            );
        }

        Ok(descriptor)
    }

    fn create_rtv(
        &mut self,
        device: &ID3D12Device4,
        descriptor_manager: &mut DescriptorManager,
        texture: &Texture,
    ) -> Result<DescriptorHandle> {
        let descriptor = descriptor_manager.allocate(DescriptorType::RenderTargetView)?;

        let (view_dimension, anonymous_member) = match texture.info.dimension {
            TextureDimension::One(_) => {
                if texture.info.array_size > 1 {
                    (
                        D3D12_RTV_DIMENSION_TEXTURE1DARRAY,
                        D3D12_RENDER_TARGET_VIEW_DESC_0 {
                            Texture1DArray: D3D12_TEX1D_ARRAY_RTV {
                                FirstArraySlice: 0,
                                ArraySize: texture.info.array_size as u32,
                                MipSlice: 0,
                            },
                        },
                    )
                } else {
                    (
                        D3D12_RTV_DIMENSION_TEXTURE1D,
                        D3D12_RENDER_TARGET_VIEW_DESC_0 {
                            Texture1D: D3D12_TEX1D_RTV { MipSlice: 0 },
                        },
                    )
                }
            }
            TextureDimension::Two(_, _) => {
                if texture.info.array_size > 1 {
                    (
                        D3D12_RTV_DIMENSION_TEXTURE2DARRAY,
                        D3D12_RENDER_TARGET_VIEW_DESC_0 {
                            Texture2DArray: D3D12_TEX2D_ARRAY_RTV {
                                FirstArraySlice: 0,
                                ArraySize: texture.info.array_size as u32,
                                PlaneSlice: 0,
                                MipSlice: 0,
                            },
                        },
                    )
                } else {
                    (
                        D3D12_RTV_DIMENSION_TEXTURE2D,
                        D3D12_RENDER_TARGET_VIEW_DESC_0 {
                            Texture2D: D3D12_TEX2D_RTV {
                                PlaneSlice: 0,
                                MipSlice: 0,
                            },
                        },
                    )
                }
            }
            TextureDimension::Three(_, _, _) => (
                D3D12_RTV_DIMENSION_TEXTURE3D,
                D3D12_RENDER_TARGET_VIEW_DESC_0 {
                    Texture3D: D3D12_TEX3D_RTV {
                        MipSlice: 0,
                        FirstWSlice: 0,
                        WSize: u32::MAX,
                    },
                },
            ),
        };

        unsafe {
            device.CreateRenderTargetView(
                &texture.resource.device_resource,
                &D3D12_RENDER_TARGET_VIEW_DESC {
                    Format: texture.info.format,
                    ViewDimension: view_dimension,
                    Anonymous: anonymous_member,
                },
                descriptor_manager.get_cpu_handle(&descriptor)?,
            );
        }

        Ok(descriptor)
    }

    fn create_srv(
        &mut self,
        device: &ID3D12Device4,
        descriptor_manager: &mut DescriptorManager,
        texture: &Texture,
    ) -> Result<DescriptorHandle> {
        let descriptor = descriptor_manager.allocate(DescriptorType::Resource)?;
        let (view_dimension, anonymous_member) = match texture.info.dimension {
            TextureDimension::One(_) => {
                if texture.info.array_size > 1 {
                    (
                        D3D12_SRV_DIMENSION_TEXTURE1DARRAY,
                        D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                            Texture1DArray: D3D12_TEX1D_ARRAY_SRV {
                                MostDetailedMip: 0,
                                MipLevels: texture.info.num_mips as u32,
                                FirstArraySlice: 0,
                                ArraySize: texture.info.array_size as u32,
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
                                MipLevels: texture.info.num_mips as u32,
                                ResourceMinLODClamp: 0.0,
                            },
                        },
                    )
                }
            }
            TextureDimension::Two(_, _) => {
                if texture.info.array_size > 1 {
                    (
                        D3D12_SRV_DIMENSION_TEXTURE2DARRAY,
                        D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                            Texture2DArray: D3D12_TEX2D_ARRAY_SRV {
                                MostDetailedMip: 0,
                                MipLevels: texture.info.num_mips as u32,
                                FirstArraySlice: 0,
                                ArraySize: texture.info.array_size as u32,
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
                                MipLevels: texture.info.num_mips as u32,
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
                        MipLevels: texture.info.num_mips as u32,
                        ResourceMinLODClamp: 0.0,
                    },
                },
            ),
        };

        unsafe {
            device.CreateShaderResourceView(
                &texture.resource.device_resource,
                &D3D12_SHADER_RESOURCE_VIEW_DESC {
                    Format: texture.info.format,
                    ViewDimension: view_dimension,
                    Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                    Anonymous: anonymous_member,
                },
                descriptor_manager.get_cpu_handle(&descriptor)?,
            );
        }

        Ok(descriptor)
    }

    pub fn get_srv(&self, handle: &TextureHandle) -> Result<DescriptorHandle> {
        let srv_index = handle.srv_index.context("No SRV for texture")?;
        self.srv_descriptors
            .get(srv_index)
            .copied()
            .context("Invalid rtv index")
    }
}
