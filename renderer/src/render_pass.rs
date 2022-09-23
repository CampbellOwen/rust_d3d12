use anyhow::Result;
use windows::Win32::Graphics::Direct3D12::ID3D12GraphicsCommandList;

use crate::object::Object;

pub mod bindless_texture_pass;

pub struct Resources {}

pub trait RenderPass {
    fn render(
        command_list: &ID3D12GraphicsCommandList,
        resources: &mut Resources,
        objects: &[Object],
    ) -> Result<()>;
}
