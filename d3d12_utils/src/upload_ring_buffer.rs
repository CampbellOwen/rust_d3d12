use anyhow::{ensure, Result};
use windows::{
    core::PCWSTR,
    Win32::Graphics::{Direct3D12::*, Dxgi::Common::DXGI_SAMPLE_DESC},
};

use crate::{align_data, CommandQueue, Heap, Resource, SubResource};

struct Submission {
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList1,
    fence_value: u64,
    offset: usize,
    size: usize,
}

impl Submission {
    pub fn new(device: &ID3D12Device4) -> Result<Self> {
        let command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_COPY) }?;

        let command_list: ID3D12GraphicsCommandList1 = unsafe {
            device.CreateCommandList1(
                0,
                D3D12_COMMAND_LIST_TYPE_COPY,
                D3D12_COMMAND_LIST_FLAG_NONE,
            )
        }?;

        unsafe {
            command_list.SetName(PCWSTR::from(&"Upload Command List".into()))?;
        }

        Ok(Self {
            command_allocator,
            command_list,
            fence_value: 0,
            offset: 0,
            size: 0,
        })
    }
}

const MAX_NUMBER_SUBMISSIONS: usize = 16;
pub struct UploadRingBuffer {
    buffer_size: usize,
    buffer: Resource,

    buffer_head: usize,
    buffer_tail: usize,

    submissions: [Submission; MAX_NUMBER_SUBMISSIONS],
    submissions_head: usize,
    submissions_used: usize,

    upload_queue: CommandQueue,
}

pub struct Upload<'resource> {
    sub_resource: SubResource<'resource>,
    submission: &'resource mut Submission,
}

impl UploadRingBuffer {
    pub fn new(
        device: &ID3D12Device4,
        upload_heap: Option<&mut Heap>,
        size: Option<usize>,
    ) -> Result<UploadRingBuffer> {
        let size = size.unwrap_or(64 * 1024 * 1024);

        let buffer_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Alignment: 0,
            Width: size as u64,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            ..Default::default()
        };

        let buffer = if let Some(heap) = upload_heap {
            heap.create_resource(device, &buffer_desc, D3D12_RESOURCE_STATE_COMMON, true)?
        } else {
            Resource::create_committed(
                device,
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    ..Default::default()
                },
                &buffer_desc,
                D3D12_RESOURCE_STATE_COMMON,
                true,
            )?
        };

        let submissions =
            array_init::try_array_init(|_| -> Result<Submission> { Submission::new(device) })?;

        let upload_queue = CommandQueue::new(device, D3D12_COMMAND_LIST_TYPE_COPY)?;

        Ok(UploadRingBuffer {
            buffer_size: size,
            buffer,
            submissions,

            buffer_head: 0,
            buffer_tail: 0,

            submissions_head: 0,
            submissions_used: 0,

            upload_queue,
        })
    }

    pub fn allocate(&mut self, size: usize) -> Result<Upload> {
        let size = align_data(size, D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT as usize);
        ensure!(self.submissions_used < MAX_NUMBER_SUBMISSIONS);
        ensure!(size < self.buffer_size);
        ensure!((self.buffer_head + size < self.buffer_size) || size < self.buffer_tail);

        let offset = if self.buffer_head + size > self.buffer_size {
            0
        } else {
            self.buffer_head
        };

        self.buffer_head = offset + size;

        let submission_index =
            (self.submissions_head + self.submissions_used) % self.submissions.len();
        self.submissions_used += 1;

        let submission = &mut self.submissions[submission_index];
        unsafe {
            submission.command_allocator.Reset()?;

            submission
                .command_list
                .Reset(&submission.command_allocator, None)?;
        }

        Ok(Upload {
            sub_resource: self.buffer.create_sub_resource(size, offset)?,
            submission,
        })
    }

    pub fn submit(&mut self, upload: Upload, dependent_queue: Option<&CommandQueue>) -> Result<()> {
        let fence_value = self
            .upload_queue
            .execute_command_list(&upload.submission.command_list.clone().into())?;
        upload.submission.fence_value = fence_value;

        if let Some(queue) = dependent_queue {
            queue.insert_wait_for_queue_fence(&self.upload_queue, fence_value)?;
        }

        Ok(())
    }
}
