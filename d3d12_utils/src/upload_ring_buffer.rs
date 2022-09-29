use anyhow::{ensure, Result};
use windows::{
    core::PCWSTR,
    Win32::Graphics::{Direct3D12::*, Dxgi::Common::DXGI_SAMPLE_DESC},
};

use crate::{align_data, CommandQueue, Heap, Resource, SubResource};

#[derive(Debug)]
struct Submission {
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList1,
    fence_value: u64,
    offset: usize,
    size: usize,
    padding: usize,
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
            padding: 0,
        })
    }

    pub fn reset(&mut self) {
        self.fence_value = 0;
        self.offset = 0;
        self.size = 0;
        self.padding = 0;
    }
}

const MAX_NUMBER_SUBMISSIONS: usize = 16;
#[derive(Debug)]
pub struct UploadRingBuffer {
    buffer_size: usize,
    buffer: Resource,

    buffer_head: usize,
    buffer_tail: usize,

    submissions: [Submission; MAX_NUMBER_SUBMISSIONS],
    submissions_start: usize,
    submissions_used: usize,

    upload_queue: CommandQueue,
}

pub struct Upload<'resource> {
    pub sub_resource: SubResource<'resource>,
    submission: &'resource mut Submission,
    pub command_list: ID3D12GraphicsCommandList1,
    upload_queue: &'resource mut CommandQueue,
}

impl<'a> Upload<'a> {
    pub fn submit(self, dependent_queue: Option<&CommandQueue>) -> Result<()> {
        unsafe {
            self.submission.command_list.Close()?;
        }
        let fence_value = self
            .upload_queue
            .execute_command_list(&self.submission.command_list.clone().into())?;
        self.submission.fence_value = fence_value;

        if let Some(queue) = dependent_queue {
            queue.insert_wait_for_queue_fence(self.upload_queue, fence_value)?;
        }

        Ok(())
    }
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
            heap.create_resource(
                device,
                &buffer_desc,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                true,
            )?
        } else {
            Resource::create_committed(
                device,
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    ..Default::default()
                },
                &buffer_desc,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                true,
            )?
        };

        let submissions =
            array_init::try_array_init(|_| -> Result<Submission> { Submission::new(device) })?;

        let upload_queue = CommandQueue::new(
            device,
            D3D12_COMMAND_LIST_TYPE_COPY,
            "Upload Ring Buffer Copy Command Queue",
        )?;

        Ok(UploadRingBuffer {
            buffer_size: size,
            buffer,
            submissions,

            buffer_head: 0,
            buffer_tail: 0,

            submissions_start: 0,
            submissions_used: 0,

            upload_queue,
        })
    }

    pub fn allocate(&mut self, size: usize) -> Result<Upload> {
        let raw_size = size; // Keep track of the actual size of the user data
        let size = align_data(size, D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT as usize);

        if self.submissions_used >= MAX_NUMBER_SUBMISSIONS {
            self.clean_up_submissions()?;
        }

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
            (self.submissions_start + self.submissions_used) % self.submissions.len();
        self.submissions_used += 1;

        let submission = &mut self.submissions[submission_index];
        unsafe {
            submission.command_allocator.Reset()?;

            submission
                .command_list
                .Reset(&submission.command_allocator, None)?;
        }
        submission.offset = offset;
        submission.padding = size - raw_size;
        submission.size = raw_size;

        let command_list = submission.command_list.clone();
        Ok(Upload {
            sub_resource: self.buffer.create_sub_resource(raw_size, offset)?,
            submission,
            command_list,
            upload_queue: &mut self.upload_queue,
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

    pub fn clean_up_submissions(&mut self) -> Result<()> {
        let start_idx = self.submissions_start;
        let num_submissions = self.submissions_used;
        for i in 0..num_submissions {
            let index = (start_idx + i) % MAX_NUMBER_SUBMISSIONS;

            let submission = &mut self.submissions[index];
            let fence = submission.fence_value;
            if self.upload_queue.is_fence_complete(fence) {
                ensure!(self.buffer_tail == submission.offset);

                if self.buffer_tail + submission.size + submission.padding > self.buffer_size {
                    self.buffer_tail = 0;
                } else {
                    self.buffer_tail += submission.size + submission.padding;
                }

                self.submissions_start = (self.submissions_start + 1) % MAX_NUMBER_SUBMISSIONS;
                self.submissions_used -= 1;

                submission.reset();
            } else {
                return Ok(());
            }
        }

        Ok(())
    }

    pub fn wait_on_pending(&mut self) {
        todo!()
    }
}
