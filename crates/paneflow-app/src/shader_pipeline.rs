// US-008/009: Terminal GPU rendering pipeline
//
// Manages two wgpu render pipelines (background quads + glyph quads),
// instance buffers, uniform buffer, and the glyph atlas.

use crate::glyph_atlas::{GlyphAtlas, GlyphKey};
use crate::renderer::{CellData, TerminalGrid};
use iced::widget::shader::wgpu;
use std::hash::{Hash, Hasher};

// ─── Instance data ──────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BgInstance {
    pub cell_pos: [f32; 2],
    pub cell_size: [f32; 2],
    pub bg_color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlyphInstance {
    pub cell_pos: [f32; 2],
    pub glyph_size: [f32; 2],
    pub glyph_offset: [f32; 2],
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub fg_color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    viewport_size: [f32; 2],
    _padding: [f32; 2],
}

// ─── Pipeline ───────────────────────────────────────────────────────────────

pub struct TerminalPipeline {
    bg_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    bg_buffer: wgpu::Buffer,
    glyph_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    pub atlas: GlyphAtlas,
    bg_count: u32,
    glyph_count: u32,
    bg_capacity: u64,
    glyph_capacity: u64,
    /// US-010: Last viewport + grid dimensions used — skip re-upload if unchanged
    last_viewport: [f32; 2],
    last_grid_hash: u64,
}

impl TerminalPipeline {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_size: f32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terminal_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/terminal.wgsl").into()),
        });

        // Uniform buffer
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terminal_uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create atlas (needs a dummy layout for initialization)
        let dummy_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dummy"),
            entries: &[],
        });
        let mut atlas = GlyphAtlas::new(device, &dummy_layout, font_size);
        atlas.rebuild_bind_group(device, &uniform_buffer);
        atlas.pre_warm(device, queue);

        let bind_group_layout = atlas.bind_group_layout();

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terminal_pipeline_layout"),
            bind_group_layouts: &[bind_group_layout],
            push_constant_ranges: &[],
        });

        // Background pipeline (no blending)
        let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bg_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_bg",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<BgInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        // cell_pos
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        // cell_size
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        // bg_color
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 2,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_bg",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
        });

        // Glyph pipeline (alpha blending)
        let glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glyph_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_glyph",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        // cell_pos
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        // glyph_size
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        // glyph_offset
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 16,
                            shader_location: 2,
                        },
                        // uv_min
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 24,
                            shader_location: 3,
                        },
                        // uv_max
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 32,
                            shader_location: 4,
                        },
                        // fg_color
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 40,
                            shader_location: 5,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_glyph",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
        });

        // Initial instance buffers
        let initial_capacity: u64 = 4096;
        let bg_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bg_instances"),
            size: initial_capacity * std::mem::size_of::<BgInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph_instances"),
            size: initial_capacity * std::mem::size_of::<GlyphInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            bg_pipeline,
            glyph_pipeline,
            bg_buffer,
            glyph_buffer,
            uniform_buffer,
            atlas,
            bg_count: 0,
            glyph_count: 0,
            bg_capacity: initial_capacity,
            glyph_capacity: initial_capacity,
            last_viewport: [0.0; 2],
            last_grid_hash: 0,
        }
    }

    /// Update instance buffers from a terminal grid.
    /// US-010: Skips re-upload if grid content hasn't changed (dirty-frame optimization).
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &TerminalGrid,
        viewport: [f32; 2],
    ) {
        // US-010: Fast hash to detect unchanged frames — zero upload on idle terminal
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        grid.rows.hash(&mut hasher);
        grid.cols.hash(&mut hasher);
        grid.cursor_row.hash(&mut hasher);
        grid.cursor_col.hash(&mut hasher);
        for cell in &grid.cells {
            (cell.character as u32).hash(&mut hasher);
            cell.fg.r.to_bits().hash(&mut hasher);
            cell.fg.g.to_bits().hash(&mut hasher);
            cell.fg.b.to_bits().hash(&mut hasher);
            cell.bg.r.to_bits().hash(&mut hasher);
            cell.bg.g.to_bits().hash(&mut hasher);
            cell.bg.b.to_bits().hash(&mut hasher);
            cell.bold.hash(&mut hasher);
        }
        let grid_hash = hasher.finish();

        if grid_hash == self.last_grid_hash && viewport == self.last_viewport {
            return; // No changes — skip GPU upload
        }
        self.last_grid_hash = grid_hash;
        self.last_viewport = viewport;

        let (cell_w, cell_h) = self.atlas.cell_size();
        let default_bg = CellData::default().bg;

        // Update uniforms
        let uniforms = Uniforms {
            viewport_size: viewport,
            _padding: [0.0; 2],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Rebuild bind group if atlas texture was replaced (growth)
        self.atlas.ensure_bind_group(device, &self.uniform_buffer);

        // Build instance data
        let mut bg_instances = Vec::with_capacity(grid.rows * grid.cols);
        let mut glyph_instances = Vec::with_capacity(grid.rows * grid.cols);

        for row in 0..grid.rows {
            for col in 0..grid.cols {
                let cell = grid.cell(row, col);
                let x = col as f32 * cell_w;
                let y = row as f32 * cell_h;

                // Background instance (skip default bg to save bandwidth)
                if cell.bg != default_bg {
                    bg_instances.push(BgInstance {
                        cell_pos: [x, y],
                        cell_size: [cell_w, cell_h],
                        bg_color: [cell.bg.r, cell.bg.g, cell.bg.b, cell.bg.a],
                    });
                }

                // Glyph instance (skip spaces)
                if cell.character != ' ' && cell.character != '\0' {
                    let key = GlyphKey {
                        ch: cell.character,
                        bold: cell.bold,
                        italic: cell.italic,
                    };
                    if let Some(entry) = self.atlas.get_or_insert(device, queue, key) {
                        glyph_instances.push(GlyphInstance {
                            cell_pos: [x, y],
                            glyph_size: [entry.metrics.width, entry.metrics.height],
                            glyph_offset: [
                                entry.metrics.bearing_x,
                                cell_h - entry.metrics.bearing_y,
                            ],
                            uv_min: [entry.uv[0], entry.uv[1]],
                            uv_max: [entry.uv[2], entry.uv[3]],
                            fg_color: [cell.fg.r, cell.fg.g, cell.fg.b, cell.fg.a],
                        });
                    }
                }
            }
        }

        // Grow buffers if needed
        let bg_needed = bg_instances.len() as u64;
        if bg_needed > self.bg_capacity {
            self.bg_capacity = bg_needed.next_power_of_two();
            self.bg_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bg_instances"),
                size: self.bg_capacity * std::mem::size_of::<BgInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        let glyph_needed = glyph_instances.len() as u64;
        if glyph_needed > self.glyph_capacity {
            self.glyph_capacity = glyph_needed.next_power_of_two();
            self.glyph_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("glyph_instances"),
                size: self.glyph_capacity * std::mem::size_of::<GlyphInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        // Upload
        if !bg_instances.is_empty() {
            queue.write_buffer(&self.bg_buffer, 0, bytemuck::cast_slice(&bg_instances));
        }
        if !glyph_instances.is_empty() {
            queue.write_buffer(
                &self.glyph_buffer,
                0,
                bytemuck::cast_slice(&glyph_instances),
            );
        }

        self.bg_count = bg_instances.len() as u32;
        self.glyph_count = glyph_instances.len() as u32;
    }

    /// Issue draw calls for background and glyph passes.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: iced::Rectangle<u32>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("terminal_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_scissor_rect(clip.x, clip.y, clip.width, clip.height);

        // Pass 1: Background quads
        if self.bg_count > 0 {
            pass.set_pipeline(&self.bg_pipeline);
            pass.set_bind_group(0, self.atlas.bind_group(), &[]);
            pass.set_vertex_buffer(0, self.bg_buffer.slice(..));
            pass.draw(0..6, 0..self.bg_count);
        }

        // Pass 2: Glyph quads
        if self.glyph_count > 0 {
            pass.set_pipeline(&self.glyph_pipeline);
            pass.set_bind_group(0, self.atlas.bind_group(), &[]);
            pass.set_vertex_buffer(0, self.glyph_buffer.slice(..));
            pass.draw(0..6, 0..self.glyph_count);
        }
    }
}

impl std::fmt::Debug for TerminalPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalPipeline")
            .field("bg_count", &self.bg_count)
            .field("glyph_count", &self.glyph_count)
            .field("atlas", &self.atlas)
            .finish()
    }
}
