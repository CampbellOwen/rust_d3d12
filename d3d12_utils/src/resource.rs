use std::ffi::c_void;

use anyhow::{ensure, Context, Result};
use windows::Win32::Graphics::Direct3D12::*;

#[derive(Debug)]
pub struct Resource {
    pub device_resource: ID3D12Resource,
    pub size: usize,
    pub mapped_data: *mut c_void,
}

#[derive(Debug)]
pub struct SubResource<'resource> {
    resource: &'resource Resource,
    size: usize,
    offset: usize,
}

impl<'resource> SubResource<'resource> {
    pub fn get_mapped_data(&self) -> Option<*mut c_void> {
        if self.resource.mapped_data.is_null() {
            return None;
        }

        unsafe { Some(self.resource.mapped_data.add(self.offset) as _) }
    }

    pub fn copy_from<T: Sized>(&self, data: &[T]) -> Result<()> {
        let data_size_bytes = std::mem::size_of_val(data);
        ensure!(data_size_bytes <= self.size, "Resource is not big enough");

        let mapped_data = self.get_mapped_data().context("Data not mapped")?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr() as *mut u8,
                mapped_data as *mut u8,
                data_size_bytes,
            );
        }

        Ok(())
    }
}

impl Resource {
    pub fn create_committed(
        device: &ID3D12Device4,
        heap_properties: &D3D12_HEAP_PROPERTIES,
        desc: &D3D12_RESOURCE_DESC,
        initial_state: D3D12_RESOURCE_STATES,
        mapped: bool,
    ) -> Result<Self> {
        let mut resource: Option<ID3D12Resource> = None;

        unsafe {
            device.CreateCommittedResource(
                heap_properties,
                D3D12_HEAP_FLAG_NONE,
                desc,
                initial_state,
                std::ptr::null(),
                &mut resource,
            )?;
        }

        let resource = resource.unwrap();

        let mut p_data = std::ptr::null_mut();
        if mapped {
            unsafe {
                resource.Map(0, std::ptr::null(), &mut p_data)?;
            }
        }
        Ok(Resource {
            device_resource: resource,
            size: desc.Width as usize * desc.Height as usize,
            mapped_data: p_data,
        })
    }
    pub fn copy_from<T: Sized>(&self, data: &[T]) -> Result<()> {
        let data_size_bytes = std::mem::size_of_val(data);
        ensure!(!self.mapped_data.is_null(), "Resoure is not mapped");
        ensure!(data_size_bytes <= self.size, "Resource is not big enough");

        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr() as *mut u8,
                self.mapped_data as *mut u8,
                data_size_bytes,
            );
        }

        Ok(())
    }

    pub fn gpu_address(&self) -> u64 {
        unsafe { self.device_resource.GetGPUVirtualAddress() }
    }

    pub fn create_sub_resource(&self, size: usize, offset: usize) -> Result<SubResource> {
        ensure!((offset + size) <= self.size);

        Ok(SubResource {
            resource: self,
            size,
            offset,
        })
    }
}

impl Drop for Resource {
    fn drop(&mut self) {
        if !self.mapped_data.is_null() {
            unsafe {
                self.device_resource.Unmap(0, std::ptr::null());
            }
        }
    }
}
