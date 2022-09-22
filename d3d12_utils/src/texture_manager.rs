use crate::{CommandQueue, Descriptor, DescriptorManager, DescriptorType, Heap, UploadRingBuffer};
use anyhow::{ensure, Context, Result};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

const DEFAULT_TEXTURE_HEAP_SIZE: usize = 2160 * 3840 * 4 * 100;

#[derive(Debug, Clone, Copy)]
pub enum TextureType {
    TwoDim(usize, u32),
    ThreeDim(usize, u32, u16),
}
#[derive(Debug)]
pub struct Texture {
    pub texture_type: TextureType,
    pub num_mips: u32,
    pub resource: ID3D12Resource,
}

#[derive(Debug)]
pub struct TextureManager {
    texture_heap: Heap,
    rtv_descriptors: Vec<Descriptor>,
    srv_descriptors: Vec<Descriptor>,
    uav_descriptors: Vec<Descriptor>,
    textures: Vec<Texture>,
}

#[derive(Debug, Clone, Copy)]
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
        device: &ID3D12Device4,
        uploader: &mut UploadRingBuffer,
        dependent_queue: Option<&CommandQueue>,
        texture_type: TextureType,
        format: DXGI_FORMAT,
        pixel_size_bytes: u8,
        data: &[&[u8]],
    ) -> Result<(u64, TextureHandle)> {
        let num_subresources = data.len();

        ensure!(num_subresources <= MAX_NUM_SUBRESOURCES);
        let (dimension, width, height, depth) = match texture_type {
            TextureType::TwoDim(width, height) => {
                (D3D12_RESOURCE_DIMENSION_TEXTURE2D, width, height, 1)
            }
            TextureType::ThreeDim(width, height, depth) => {
                (D3D12_RESOURCE_DIMENSION_TEXTURE3D, width, height, depth)
            }
        };

        let texture_desc = D3D12_RESOURCE_DESC {
            Dimension: dimension,
            Width: width as u64,
            Height: height as u32,
            DepthOrArraySize: depth as u16,
            MipLevels: data.len() as u16,
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
        let mut row_size_bytes = 0;
        let mut total_bytes = 0;

        unsafe {
            device.GetCopyableFootprints(
                &texture_desc,
                0,
                num_subresources as u32,
                0,
                layouts.as_mut_ptr(),
                num_rows.as_mut_ptr(),
                &mut row_size_bytes,
                &mut total_bytes,
            );
        }

        let upload_context = uploader.allocate(total_bytes as usize)?;

        for layout in &layouts[0..num_subresources] {
            for z in 0..layout.Footprint.Depth {
                for row in 0..layout.Footprint.Height {}
            }
        }

        upload_context.submit(dependent_queue)?;

        todo!()
    }

    pub fn get_texture(&self, handle: &TextureHandle) -> Result<&Texture> {
        self.textures
            .get(handle.index)
            .context("Invalid texture handle")
    }

    pub fn get_rtv(
        &mut self,
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
        self.srv_descriptors.push(descriptor);
        handle.srv_index = Some(self.srv_descriptors.len() - 1);

        Ok(descriptor)
    }
}
