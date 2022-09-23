# D3D12 Renderer

## TODO

- [x] Load mesh
  - [x] Matrices
  - [x] Proper vertex buffers (staging heap, then copy to non-cpu visible heap)
  - [x] Index buffer
  - [x] Constant buffers
- [x] Depth buffer
- [x] Upload Queue
  - [x] Basic usage
  - [x] Ring buffer based upload queue
- [x] Resources
  - [x] Sub-allocate from a resource (for constant buffers)
- [x] Textures
- [x] Bindless shaders
- [ ] Render Pass
- [ ] Mesh abstraction
- [ ] Precompiled shaders
- [ ] Shader reflection
  - [ ] Parse vertex format from DXIL?
- [ ] Basic Shadows

## Progress

2022-08-21

![image of bunny](img/2022-08-21.png)

2022-09-19

Uploaded my first texture
![image of bunny with texture](img/2022-09-19.png)