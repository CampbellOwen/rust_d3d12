use anyhow::{ensure, Result};
use windows::Win32::Graphics::Direct3D12::*;

#[derive(Debug)]
pub struct DescriptorHeap {
    pub heap: ID3D12DescriptorHeap,
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

    pub fn shader_resource_view_heap(
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

    pub fn allocate_handle(&mut self) -> Result<(u32, D3D12_CPU_DESCRIPTOR_HANDLE)> {
        ensure!(
            self.num_allocated < self.num_descriptors,
            "Not enough descriptors"
        );

        let heap_start_handle = unsafe { self.heap.GetCPUDescriptorHandleForHeapStart() };
        let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: heap_start_handle.ptr + self.num_allocated as usize * self.descriptor_size,
        };

        self.num_allocated += 1;

        Ok((self.num_allocated - 1, handle))
    }

    pub fn get_cpu_handle(&self, index: u32) -> Result<D3D12_CPU_DESCRIPTOR_HANDLE> {
        ensure!(index < self.num_allocated, "index out of bounds");

        let heap_start_handle = unsafe { self.heap.GetCPUDescriptorHandleForHeapStart() };
        Ok(D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: heap_start_handle.ptr + (index as usize * self.descriptor_size),
        })
    }

    pub fn get_gpu_handle(&self, index: u32) -> Result<D3D12_GPU_DESCRIPTOR_HANDLE> {
        ensure!(index < self.num_allocated, "index out of bounds");

        let heap_start_handle = unsafe { self.heap.GetGPUDescriptorHandleForHeapStart() };
        Ok(D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: heap_start_handle.ptr + (index as u64 * self.descriptor_size as u64),
        })
    }
}
