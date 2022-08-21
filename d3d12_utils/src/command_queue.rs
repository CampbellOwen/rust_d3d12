use anyhow::Result;
use windows::Win32::{
    Foundation::HANDLE,
    Graphics::Direct3D12::*,
    System::{
        Threading::{CreateEventA, WaitForSingleObject},
        WindowsProgramming::INFINITE,
    },
};

#[derive(Debug)]
pub struct CommandQueue {
    pub queue: ID3D12CommandQueue,

    fence: ID3D12Fence,
    last_fence_value: u64,
    next_fence_value: u64,
    fence_event: HANDLE,
}

impl CommandQueue {
    pub fn new(
        device: &ID3D12Device4,
        command_type: D3D12_COMMAND_LIST_TYPE,
    ) -> Result<CommandQueue> {
        let queue = unsafe {
            device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: command_type,
                ..Default::default()
            })
        }?;

        // https://alextardif.com/D3D11To12P1.html
        let last_fence_value = (command_type.0 as u64) << 56;
        let next_fence_value = last_fence_value + 1;

        let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }?;
        let fence_event = unsafe { CreateEventA(std::ptr::null(), false, false, None) }?;

        unsafe {
            fence.Signal(last_fence_value)?;
        }

        Ok(CommandQueue {
            queue,
            fence,
            last_fence_value,
            next_fence_value,
            fence_event,
        })
    }

    /// fence.GetCompletedValue can be expensive, try not to call this
    fn poll_fence_value(&mut self) -> u64 {
        self.last_fence_value = u64::max(
            unsafe { self.fence.GetCompletedValue() },
            self.last_fence_value,
        );

        self.last_fence_value
    }

    pub fn is_fence_complete(&mut self, fence_value: u64) -> bool {
        if fence_value > self.last_fence_value {
            self.poll_fence_value();
        }

        fence_value <= self.last_fence_value
    }

    pub fn insert_wait(&self, fence_value: u64) -> Result<()> {
        unsafe {
            self.queue.Wait(&self.fence, fence_value)?;
        }

        Ok(())
    }

    pub fn insert_wait_for_queue_fence(
        &self,
        queue: &CommandQueue,
        fence_value: u64,
    ) -> Result<()> {
        unsafe { self.queue.Wait(&queue.fence, fence_value)? }
        Ok(())
    }

    pub fn wait_for_fence_blocking(&mut self, fence_value: u64) -> Result<()> {
        if self.is_fence_complete(fence_value) {
            return Ok(());
        }

        unsafe {
            self.fence
                .SetEventOnCompletion(fence_value, self.fence_event)?;

            WaitForSingleObject(self.fence_event, INFINITE);

            self.last_fence_value = fence_value;
        }

        Ok(())
    }

    pub fn execute_command_list(&mut self, command_list: &ID3D12CommandList) -> Result<u64> {
        let value_to_signal = self.next_fence_value;
        unsafe {
            self.queue
                .ExecuteCommandLists(&[Some(command_list.clone())]);

            self.queue.Signal(&self.fence, value_to_signal)?;
        }

        self.next_fence_value += 1;

        Ok(value_to_signal)
    }

    pub fn wait_for_idle(&mut self) -> Result<()> {
        self.wait_for_fence_blocking(self.next_fence_value - 1)
    }
}
