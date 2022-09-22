use crate::DescriptorHeap;
use anyhow::{ensure, Context, Result};
use windows::Win32::Graphics::Direct3D12::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd)]
pub enum DescriptorType {
    Unset,
    Resource,
    DepthStencilView,
    RenderTargetView,
}
impl Default for DescriptorType {
    fn default() -> Self {
        DescriptorType::Unset
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Descriptor {
    tag: DescriptorType,
    index: usize,
}

#[derive(Debug)]
pub struct DescriptorManager {
    resource_descriptor_heap: DescriptorHeap,
    depth_stencil_view_heap: DescriptorHeap,
    render_target_view_heap: DescriptorHeap,

    resource_free_list: Vec<usize>,
    dsv_free_list: Vec<usize>,
    rtv_free_list: Vec<usize>,
}

fn get_handle(heap: &mut DescriptorHeap, free_list: &mut Vec<usize>) -> Result<usize> {
    if !free_list.is_empty() {
        return free_list.pop().context("Retrieving index from free list");
    }

    let (index, _) = heap.allocate_handle()?;
    Ok(index)
}

impl DescriptorManager {
    pub fn new(device: &ID3D12Device4) -> Result<Self> {
        Ok(DescriptorManager {
            resource_descriptor_heap: DescriptorHeap::resource_descriptor_heap(device, 500_000)?,
            depth_stencil_view_heap: DescriptorHeap::depth_stencil_view_heap(device, 1000)?,
            render_target_view_heap: DescriptorHeap::render_target_view_heap(device, 1000)?,

            resource_free_list: Vec::new(),
            dsv_free_list: Vec::new(),
            rtv_free_list: Vec::new(),
        })
    }

    pub fn allocate(&mut self, descriptor_type: DescriptorType) -> Result<Descriptor> {
        ensure!(descriptor_type != DescriptorType::Unset);
        let index = match descriptor_type {
            DescriptorType::Unset => None.context("Invalid descriptor type"),
            DescriptorType::Resource => get_handle(
                &mut self.resource_descriptor_heap,
                &mut self.resource_free_list,
            ),
            DescriptorType::DepthStencilView => {
                get_handle(&mut self.depth_stencil_view_heap, &mut self.dsv_free_list)
            }
            DescriptorType::RenderTargetView => {
                get_handle(&mut self.render_target_view_heap, &mut self.rtv_free_list)
            }
        }?;

        Ok(Descriptor {
            tag: descriptor_type,
            index,
        })
    }

    pub fn free(&mut self, descriptor: Descriptor) {
        match descriptor.tag {
            DescriptorType::Unset => (),
            DescriptorType::Resource => self.resource_free_list.push(descriptor.index),
            DescriptorType::DepthStencilView => self.dsv_free_list.push(descriptor.index),
            DescriptorType::RenderTargetView => self.rtv_free_list.push(descriptor.index),
        };
    }

    pub fn get_cpu_handle(&self, descriptor: &Descriptor) -> Result<D3D12_CPU_DESCRIPTOR_HANDLE> {
        match descriptor.tag {
            DescriptorType::Unset => None.context("Invalid descriptor type"),
            DescriptorType::Resource => self
                .resource_descriptor_heap
                .get_cpu_handle(descriptor.index),
            DescriptorType::DepthStencilView => self
                .depth_stencil_view_heap
                .get_cpu_handle(descriptor.index),
            DescriptorType::RenderTargetView => self
                .render_target_view_heap
                .get_cpu_handle(descriptor.index),
        }
    }

    pub fn get_gpu_handle(&self, descriptor: &Descriptor) -> Result<D3D12_GPU_DESCRIPTOR_HANDLE> {
        match descriptor.tag {
            DescriptorType::Unset => None.context("Invalid descriptor type"),
            DescriptorType::Resource => self
                .resource_descriptor_heap
                .get_gpu_handle(descriptor.index),
            DescriptorType::DepthStencilView => self
                .depth_stencil_view_heap
                .get_gpu_handle(descriptor.index),
            DescriptorType::RenderTargetView => self
                .render_target_view_heap
                .get_gpu_handle(descriptor.index),
        }
    }

    pub fn get_heap(&self, descriptor_type: DescriptorType) -> Result<ID3D12DescriptorHeap> {
        match descriptor_type {
            DescriptorType::Unset => None.context("Invalid descriptor type"),
            DescriptorType::Resource => Ok(self.resource_descriptor_heap.heap.clone()),
            DescriptorType::DepthStencilView => Ok(self.depth_stencil_view_heap.heap.clone()),
            DescriptorType::RenderTargetView => Ok(self.render_target_view_heap.heap.clone()),
        }
    }
}
