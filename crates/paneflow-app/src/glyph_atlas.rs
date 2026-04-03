// US-007: Glyph Atlas with cosmic-text + etagere
//
// Manages a GPU texture atlas for terminal glyph rendering.
// Glyphs are rasterized on-demand via cosmic-text/swash and packed
// into the atlas via etagere's shelf allocator.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use etagere::{size2, AtlasAllocator};
use iced::widget::shader::wgpu;
use std::collections::HashMap;

const INITIAL_ATLAS_SIZE: i32 = 1024;
const MAX_ATLAS_SIZE: i32 = 4096;

/// Key for looking up a glyph in the atlas cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub ch: char,
    pub bold: bool,
    pub italic: bool,
}

/// Metrics for a rasterized glyph.
#[derive(Debug, Clone, Copy)]
pub struct GlyphMetrics {
    pub width: f32,
    pub height: f32,
    pub bearing_x: f32,
    pub bearing_y: f32,
}

/// UV coordinates + metrics for a glyph in the atlas.
#[derive(Debug, Clone, Copy)]
pub struct AtlasEntry {
    pub uv: [f32; 4], // u0, v0, u1, v1
    pub metrics: GlyphMetrics,
}

/// GPU glyph texture atlas backed by cosmic-text rasterization + etagere packing.
pub struct GlyphAtlas {
    font_system: FontSystem,
    swash_cache: SwashCache,
    allocator: AtlasAllocator,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    atlas_size: i32,
    cache: HashMap<GlyphKey, AtlasEntry>,
    cell_metrics: Option<(f32, f32)>, // (cell_width, cell_height)
    font_size: f32,
    /// Set when atlas texture is replaced (growth) and bind group needs rebuild.
    bind_group_stale: bool,
}

impl GlyphAtlas {
    pub fn new(
        device: &wgpu::Device,
        uniform_bind_group_layout: &wgpu::BindGroupLayout,
        font_size: f32,
    ) -> Self {
        let size = INITIAL_ATLAS_SIZE;
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let allocator = AtlasAllocator::new(size2(size, size));

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph_atlas"),
            size: wgpu::Extent3d {
                width: size as u32,
                height: size as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glyph_atlas_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Create bind group layout for atlas texture + sampler
        // This is combined with the uniform layout in the pipeline
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas_bind_group_layout"),
            entries: &[
                // binding 0: uniforms (provided by pipeline)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // binding 1: atlas texture
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // binding 2: atlas sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Placeholder bind group — will be rebuilt when uniform buffer is available
        // The actual bind group is created in rebuild_bind_group()
        let placeholder_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("placeholder_uniform"),
            size: 16, // 2 * f32 padded to 16 bytes
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: placeholder_uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let _ = uniform_bind_group_layout; // used by pipeline, not atlas directly

        Self {
            font_system,
            swash_cache,
            allocator,
            texture,
            texture_view,
            sampler,
            bind_group_layout,
            bind_group,
            atlas_size: size,
            cache: HashMap::new(),
            cell_metrics: None,
            font_size,
            bind_group_stale: false,
        }
    }

    /// Get the bind group layout (uniform + texture + sampler).
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    /// Rebuild the bind group with a real uniform buffer.
    pub fn rebuild_bind_group(&mut self, device: &wgpu::Device, uniform_buffer: &wgpu::Buffer) {
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
    }

    /// Rebuild the bind group if the atlas texture was replaced (growth).
    pub fn ensure_bind_group(&mut self, device: &wgpu::Device, uniform_buffer: &wgpu::Buffer) {
        if self.bind_group_stale {
            self.rebuild_bind_group(device, uniform_buffer);
            self.bind_group_stale = false;
        }
    }

    /// Get the bind group for rendering.
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Get cell dimensions (width, height) computed from font metrics.
    pub fn cell_size(&mut self) -> (f32, f32) {
        if let Some(size) = self.cell_metrics {
            return size;
        }
        // Measure a reference character to determine cell size
        let metrics = Metrics::new(self.font_size, self.font_size * 1.2);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_text(
            &mut self.font_system,
            "M",
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);

        let cell_w = self.font_size * 0.6; // monospace approximation
        let cell_h = self.font_size * 1.2;
        self.cell_metrics = Some((cell_w, cell_h));
        (cell_w, cell_h)
    }

    /// Look up or rasterize a glyph, returning its atlas entry.
    pub fn get_or_insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: GlyphKey,
    ) -> Option<AtlasEntry> {
        if let Some(entry) = self.cache.get(&key) {
            return Some(*entry);
        }

        // Rasterize the glyph using cosmic-text
        let metrics = Metrics::new(self.font_size, self.font_size * 1.2);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);

        let mut attrs = Attrs::new().family(Family::Monospace);
        if key.bold {
            attrs = attrs.weight(cosmic_text::Weight::BOLD);
        }
        if key.italic {
            attrs = attrs.style(cosmic_text::Style::Italic);
        }

        let s = key.ch.to_string();
        buffer.set_text(&mut self.font_system, &s, attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, false);

        // Extract the glyph image from layout runs
        let mut glyph_image: Option<(Vec<u8>, i32, i32, i32, i32)> = None;

        if let Some(run) = buffer.layout_runs().next() {
            if let Some(glyph) = run.glyphs.iter().next() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let cache_key = physical.cache_key;

                if let Some(image) = self.swash_cache.get_image(&mut self.font_system, cache_key) {
                    if image.placement.width > 0 && image.placement.height > 0 {
                        let w = image.placement.width as i32;
                        let h = image.placement.height as i32;
                        let left = image.placement.left;
                        let top = image.placement.top;

                        // Convert to grayscale alpha if needed
                        let alpha_data: Vec<u8> = match image.content {
                            cosmic_text::SwashContent::Mask => image.data.clone(),
                            cosmic_text::SwashContent::Color => {
                                // RGBA → take alpha channel
                                image
                                    .data
                                    .chunks(4)
                                    .map(|c| c.get(3).copied().unwrap_or(0))
                                    .collect()
                            }
                            cosmic_text::SwashContent::SubpixelMask => {
                                // RGB subpixel → average to grayscale
                                image
                                    .data
                                    .chunks(3)
                                    .map(|c| {
                                        let sum: u32 = c.iter().map(|&v| v as u32).sum();
                                        (sum / c.len() as u32) as u8
                                    })
                                    .collect()
                            }
                        };

                        glyph_image = Some((alpha_data, w, h, left, top));
                    }
                }
            }
        }

        let (data, w, h, bearing_x, bearing_y) = glyph_image?;

        // Allocate space in the atlas
        let alloc = self.allocator.allocate(size2(w + 2, h + 2));
        let alloc = match alloc {
            Some(a) => a,
            None => {
                // Try to grow the atlas
                if self.atlas_size < MAX_ATLAS_SIZE {
                    return self
                        .grow_and_retry(device, queue, key, &data, w, h, bearing_x, bearing_y);
                }
                tracing::warn!("Glyph atlas full, cannot allocate for {:?}", key);
                return None;
            }
        };

        let rect = alloc.rectangle;
        let x = rect.min.x as u32 + 1; // 1px padding
        let y = rect.min.y as u32 + 1;

        // Upload to GPU texture
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(w as u32),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
        );

        let atlas_size = self.atlas_size as f32;
        let entry = AtlasEntry {
            uv: [
                x as f32 / atlas_size,
                y as f32 / atlas_size,
                (x + w as u32) as f32 / atlas_size,
                (y + h as u32) as f32 / atlas_size,
            ],
            metrics: GlyphMetrics {
                width: w as f32,
                height: h as f32,
                bearing_x: bearing_x as f32,
                bearing_y: bearing_y as f32,
            },
        };

        self.cache.insert(key, entry);
        Some(entry)
    }

    #[allow(clippy::too_many_arguments)]
    fn grow_and_retry(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: GlyphKey,
        _data: &[u8],
        _w: i32,
        _h: i32,
        _bearing_x: i32,
        _bearing_y: i32,
    ) -> Option<AtlasEntry> {
        let new_size = (self.atlas_size * 2).min(MAX_ATLAS_SIZE);
        tracing::info!(
            "Growing glyph atlas from {} to {}",
            self.atlas_size,
            new_size
        );

        self.atlas_size = new_size;
        self.allocator = AtlasAllocator::new(size2(new_size, new_size));
        self.texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph_atlas"),
            size: wgpu::Extent3d {
                width: new_size as u32,
                height: new_size as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.texture_view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_group_stale = true;

        // Re-rasterize all cached glyphs into the new atlas
        let old_keys: Vec<GlyphKey> = self.cache.keys().copied().collect();
        self.cache.clear();
        for k in old_keys {
            let _ = self.get_or_insert(device, queue, k);
        }

        // Now try the original key
        self.get_or_insert(device, queue, key)
    }

    /// Pre-warm the atlas with ASCII printable characters in regular and bold.
    pub fn pre_warm(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        for ch in ' '..='~' {
            let _ = self.get_or_insert(
                device,
                queue,
                GlyphKey {
                    ch,
                    bold: false,
                    italic: false,
                },
            );
            let _ = self.get_or_insert(
                device,
                queue,
                GlyphKey {
                    ch,
                    bold: true,
                    italic: false,
                },
            );
        }
        tracing::info!("Glyph atlas pre-warmed: {} entries", self.cache.len());
    }
}

impl std::fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("atlas_size", &self.atlas_size)
            .field("cached_glyphs", &self.cache.len())
            .finish()
    }
}
