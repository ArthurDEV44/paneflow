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
        _viewport: &shader::Viewport,
    ) {
        if !storage.has::<TerminalPipeline>() {
            storage.store(TerminalPipeline::new(device, queue, format, self.font_size));
        }

        let pipeline = storage.get_mut::<TerminalPipeline>().unwrap();
        pipeline.update(device, queue, &self.grid, [bounds.width, bounds.height]);
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        storage: &shader::Storage,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        if let Some(pipeline) = storage.get::<TerminalPipeline>() {
            pipeline.render(encoder, target, *clip_bounds);
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
