use windows::Win32::Graphics::Direct3D12::*;

use crate::object::Object;

use super::{RenderPass, Resources};

pub struct BindlessTexturePass {}

impl RenderPass for BindlessTexturePass {
    fn render(
        command_list: &ID3D12GraphicsCommandList,
        resources: &mut Resources,
        objects: &[Object],
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
