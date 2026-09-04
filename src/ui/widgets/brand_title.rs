//! Brand title widget — "S O U V E R A I N E" + French tagline.
//!
//! Used at the top of the splash and welcome screens. The title color
//! breathes via an external animation tick.

use tuie::prelude::*;

pub struct BrandTitle {
    text: Box<Text>,
}

impl DelegateWidget for BrandTitle {
    tuie::delegate_widget!(text);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl BrandTitle {
    /// Create the title with `primary` and `dim` colors.
    pub fn new(primary: Color, dim: Color) -> Box<Self> {
        let mut content = StyledString::new();
        content.push_str("\n");
        content.push_span(StyledStr::new("  S O U V E R A I N E\n").bold().fg(primary));
        content.push_span(
            StyledStr::new("  La souveraineté de la conscience\n")
                .fg(dim)
                .italic(),
        );
        content.push_str("\n");

        let mut text = Text::new().content(content);
        text.set_min_height(Some(4));

        Box::new(Self { text })
    }

    /// Update the title color for breathing animation.
    pub fn set_primary_color(&mut self, color: Color) {
        // Rebuild content with new color
        let mut content = StyledString::new();
        content.push_str("\n");
        content.push_span(StyledStr::new("  S O U V E R A I N E\n").bold().fg(color));
        content.push_span(
            StyledStr::new("  La souveraineté de la conscience\n")
                .fg(Color::BRIGHT_BLACK)
                .italic(),
        );
        content.push_str("\n");
        self.text.set_content(content);
        self.text.dirty_layout();
    }
}
