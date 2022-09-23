use anyhow::{Context, Result};
use windows::Win32::Graphics::{Direct3D12::*, Dxgi::Common::DXGI_FORMAT_R32_UINT};

use crate::{Heap, Resource};

#[derive(Debug, Default, Clone, Copy)]
pub struct MeshHandle {
    vb_index: usize,
    ib_index: usize,
    pub num_vertices: usize,
    pub vbv: Option<D3D12_VERTEX_BUFFER_VIEW>,
    pub ibv: Option<D3D12_INDEX_BUFFER_VIEW>,
}

#[derive(Debug)]
pub struct MeshManager {
    pub heap: Heap,
    vertex_buffers: Vec<Resource>,
    index_buffers: Vec<Resource>,
}

impl MeshManager {
    pub fn new(device: &ID3D12Device4) -> Result<Self> {
        Ok(MeshManager {
            heap: Heap::create_default_heap(device, 2e7 as usize, "Mesh Manager Heap")?,
            vertex_buffers: Vec::new(),
            index_buffers: Vec::new(),
        })
    }

    pub fn add(
        &mut self,
        vertex_buffer: Resource,
        index_buffer: Resource,
        vertex_buffer_stride: u32,
        num_vertices: usize,
    ) -> Result<MeshHandle> {
        let vertex_buffer_size = vertex_buffer.size;
        let index_buffer_size = index_buffer.size;
        self.vertex_buffers.push(vertex_buffer);
        self.index_buffers.push(index_buffer);

        Ok(MeshHandle {
            vb_index: self.vertex_buffers.len() - 1,
            ib_index: self.index_buffers.len() - 1,
            num_vertices,
            vbv: Some(D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: self.vertex_buffers[self.vertex_buffers.len() - 1].gpu_address(),
                StrideInBytes: vertex_buffer_stride,
                SizeInBytes: vertex_buffer_size as u32,
            }),
            ibv: Some(D3D12_INDEX_BUFFER_VIEW {
                BufferLocation: self.index_buffers[self.index_buffers.len() - 1].gpu_address(),
                SizeInBytes: index_buffer_size as u32,
                Format: DXGI_FORMAT_R32_UINT,
            }),
        })
    }

    pub fn get_buffers(&self, handle: &MeshHandle) -> Result<(&Resource, &Resource)> {
        let vertex_buffer = self
            .vertex_buffers
            .get(handle.vb_index)
            .context("Invalid vertex buffer handle")?;

        let index_buffer = self
            .index_buffers
            .get(handle.ib_index)
            .context("Invalid vertex buffer handle")?;

        Ok((vertex_buffer, index_buffer))
    }
}
