use windows::Win32::Graphics::Direct3D12::*;

use crate::object::Object;

use super::{RenderPass, Resources};

#[derive(Debug)]
pub struct BindlessTexturePass {}

impl BindlessTexturePass {
    pub fn new() -> Self {
        BindlessTexturePass {}
    }
}

impl RenderPass for BindlessTexturePass {
    fn render(
        &mut self,
        command_list: &ID3D12GraphicsCommandList,
        resources: &mut Resources,
        objects: &[Object],
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
