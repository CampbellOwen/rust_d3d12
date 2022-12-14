use anyhow::{ensure, Result};
use windows::{core::PCWSTR, Win32::Graphics::Direct3D12::*};

use crate::{align_data, Resource};

#[derive(Debug)]
pub struct Heap {
    heap: ID3D12Heap,
    size: usize,
    curr_offset: usize,
    name: String,
    num_objects: usize,
}

impl Heap {
    pub fn default_alignment() -> u32 {
        D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT
    }

    pub fn new(
        device: &ID3D12Device4,
        size: usize,
        properties: D3D12_HEAP_PROPERTIES,
        alignment: u32,
        flags: D3D12_HEAP_FLAGS,
        name: String,
    ) -> Result<Self> {
        let desc = D3D12_HEAP_DESC {
            SizeInBytes: size as u64,
            Properties: properties,
            Alignment: alignment as u64,
            Flags: flags,
        };

        let mut heap: Option<ID3D12Heap> = None;
        unsafe { device.CreateHeap(&desc, &mut heap) }?;
        let heap = heap.unwrap();

        Ok(Heap {
            heap,
            size,
            curr_offset: 0,
            name,
            num_objects: 0,
        })
    }

    pub fn create_upload_heap(device: &ID3D12Device4, size: usize, name: &str) -> Result<Self> {
        Self::new(
            device,
            size,
            D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_UPLOAD,
                ..Default::default()
            },
            D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT,
            D3D12_HEAP_FLAG_NONE,
            name.to_string(),
        )
    }

    pub fn create_default_heap(device: &ID3D12Device4, size: usize, name: &str) -> Result<Self> {
        Self::new(
            device,
            size,
            D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT,
            D3D12_HEAP_FLAG_NONE,
            name.to_string(),
        )
    }

    pub fn create_resource(
        &mut self,
        device: &ID3D12Device4,
        desc: &D3D12_RESOURCE_DESC,
        initial_state: D3D12_RESOURCE_STATES,
        clear_value: Option<D3D12_CLEAR_VALUE>,
        mapped: bool,
    ) -> Result<Resource> {
        self.num_objects += 1;

        let resource_size = desc.Width as usize * desc.Height as usize;

        let allocation_info = unsafe { device.GetResourceAllocationInfo(0, &[*desc]) };

        let aligned_offset = align_data(self.curr_offset, allocation_info.Alignment as usize);

        let total_size = (aligned_offset - self.curr_offset) + allocation_info.SizeInBytes as usize;

        ensure!(
            total_size < (self.size - self.curr_offset),
            "Not enough space in heap: {} bytes remaining, requested resource size {} bytes",
            (self.size - self.curr_offset),
            total_size
        );

        let mut resource: Option<ID3D12Resource> = None;
        unsafe {
            device.CreatePlacedResource(
                &self.heap,
                aligned_offset as u64,
                desc,
                initial_state,
                if clear_value.is_none() {
                    std::ptr::null() as _
                } else {
                    clear_value.as_ref().unwrap() as _
                },
                &mut resource,
            )?;
        }
        let resource = resource.unwrap();

        unsafe {
            resource.SetName(PCWSTR::from(
                &format!("{} - #{}", self.name, self.num_objects).into(),
            ))?;
        }

        self.curr_offset += total_size;

        let mut mapped_data = std::ptr::null_mut();

        if mapped {
            unsafe {
                resource.Map(0, std::ptr::null(), &mut mapped_data)?;
            }
        }

        Ok(Resource {
            device_resource: resource,
            size: resource_size,
            mapped_data,
        })
    }
}
