//! Offscreen rendering target for headless CI verification.

/// Color + depth textures sized to `extent`, plus a readback buffer.
pub struct OffscreenTarget {
    pub color: wgpu::Texture,
    pub depth: wgpu::Texture,
    pub extent: wgpu::Extent3d,
    readback: wgpu::Buffer,
    pitch: u32,
}

impl OffscreenTarget {
    pub fn new(device: &wgpu::Device, extent: wgpu::Extent3d) -> Self {
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-color"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        // Copy requires 256-byte aligned rows.
        let pitch = (extent.width * 4).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen-readback"),
            size: u64::from(pitch * extent.height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            color,
            depth,
            extent,
            readback,
            pitch,
        }
    }

    pub fn color_view(&self) -> wgpu::TextureView {
        self.color
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    pub fn depth_view(&self) -> wgpu::TextureView {
        self.depth
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Copies the color target into the readback buffer (call after submit).
    pub fn enqueue_copy(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            self.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.pitch),
                    rows_per_image: Some(self.extent.height),
                },
            },
            self.extent,
        );
    }

    /// Blocks until the copy lands; returns tightly packed RGBA rows.
    pub fn read_pixels(&self, device: &wgpu::Device) -> Vec<u8> {
        let (tx, rx) = std::sync::mpsc::channel();
        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().expect("map result").expect("map ok");
        let data = slice.get_mapped_range().expect("map view");
        let mut out = Vec::with_capacity((self.extent.width * self.extent.height * 4) as usize);
        for row in 0..self.extent.height {
            let start = (row * self.pitch) as usize;
            out.extend_from_slice(&data[start..start + (self.extent.width * 4) as usize]);
        }
        drop(data);
        self.readback.unmap();
        out
    }
}

/// Encodes RGBA pixels to PNG (for CI artifacts / manual inspection).
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec()).expect("pixel buffer size");
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("png encode");
    out
}
