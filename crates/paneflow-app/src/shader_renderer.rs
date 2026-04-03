// US-009: iced Shader Widget Integration
//
// Implements shader::Program and shader::Primitive for terminal rendering.
// The pipeline and atlas persist in iced's Storage across frames.

use crate::renderer::TerminalGrid;
use crate::shader_pipeline::TerminalPipeline;
use iced::widget::shader;
use iced::widget::shader::wgpu;
use iced::{mouse, Rectangle};

// ─── Primitive ──────────────────────────────────────────────────────────────

/// Per-frame data sent from draw() to prepare()/render().
#[derive(Debug)]
pub struct TerminalPrimitive {
    grid: TerminalGrid,
    font_size: f32,
}

impl shader::Primitive for TerminalPrimitive {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        storage: &mut shader::Storage,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        // Use physical pixel size for correct NDC on HiDPI displays
        let scale = viewport.scale_factor() as f32;
        let physical_w = bounds.width * scale;
        let physical_h = bounds.height * scale;

        if !storage.has::<TerminalPipeline>() {
            tracing::info!(
                format = ?format,
                font_size = self.font_size,
                "GPU pipeline: creating TerminalPipeline"
            );
            storage.store(TerminalPipeline::new(device, queue, format, self.font_size));
            tracing::info!("GPU pipeline: created successfully");
        }

        let pipeline = storage.get_mut::<TerminalPipeline>().unwrap();

        tracing::debug!(
            grid_rows = self.grid.rows,
            grid_cols = self.grid.cols,
            bounds_w = bounds.width,
            bounds_h = bounds.height,
            physical_w,
            physical_h,
            scale,
            "GPU prepare: updating pipeline"
        );

        pipeline.update(device, queue, &self.grid, [physical_w, physical_h]);
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        storage: &shader::Storage,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        if let Some(pipeline) = storage.get::<TerminalPipeline>() {
            tracing::debug!(
                clip_x = clip_bounds.x,
                clip_y = clip_bounds.y,
                clip_w = clip_bounds.width,
                clip_h = clip_bounds.height,
                bg_count = pipeline.bg_count(),
                glyph_count = pipeline.glyph_count(),
                "GPU render: issuing draw calls"
            );
            pipeline.render(encoder, target, *clip_bounds);
        } else {
            tracing::warn!("GPU render: no pipeline in storage — skipping");
        }
    }
}

// ─── Program ────────────────────────────────────────────────────────────────

/// iced Shader program for GPU terminal rendering.
pub struct TerminalShaderProgram {
    pub grid: TerminalGrid,
    pub font_size: f32,
}

impl shader::Program<crate::Message> for TerminalShaderProgram {
    type State = ();
    type Primitive = TerminalPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        TerminalPrimitive {
            grid: self.grid.clone(),
            font_size: self.font_size,
        }
    }
}
