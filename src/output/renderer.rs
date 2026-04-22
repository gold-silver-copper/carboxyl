use std::{
    io::{self, Write},
    rc::Rc,
};

use image::{DynamicImage, ImageBuffer, Rgba};
use rascii_art::{charsets, render_image_to, RenderOptions};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    gfx::{Color, Point, Rect, Size},
    input::Key,
    ui::navigation::{Navigation, NavigationAction},
    utils::log,
};

use super::{Cell, Grapheme, Painter};

pub struct Renderer {
    nav: Navigation,
    cells: Vec<(Cell, Cell)>,
    painter: Painter,
    size: Size,
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer {
            nav: Navigation::new(),
            cells: Vec::with_capacity(0),
            painter: Painter::new(),
            size: Size::new(0, 0),
        }
    }

    pub fn enable_true_color(&mut self) {
        self.painter.set_true_color(true)
    }

    pub fn keypress(&mut self, key: &Key) -> io::Result<NavigationAction> {
        let action = self.nav.keypress(key);

        Ok(action)
    }
    pub fn mouse_up(&mut self, origin: Point) -> io::Result<NavigationAction> {
        let action = self.nav.mouse_up(origin);

        Ok(action)
    }
    pub fn mouse_down(&mut self, origin: Point) -> io::Result<NavigationAction> {
        let action = self.nav.mouse_down(origin);

        Ok(action)
    }
    pub fn mouse_move(&mut self, origin: Point) -> io::Result<NavigationAction> {
        let action = self.nav.mouse_move(origin);

        Ok(action)
    }

    pub fn push_nav(&mut self, url: &str, can_go_back: bool, can_go_forward: bool) {
        self.nav.push(url, can_go_back, can_go_forward)
    }

    pub fn get_size(&self) -> Size {
        self.size
    }

    pub fn set_size(&mut self, size: Size) {
        self.nav.set_size(size);
        self.size = size;

        let mut x = 0;
        let mut y = 0;
        let bound = size.width - 1;
        let cells = (size.width + size.width * size.height) as usize;

        self.cells.clear();
        self.cells.resize_with(cells, || {
            let cell = (Cell::new(x, y), Cell::new(x, y));

            if x < bound {
                x += 1;
            } else {
                x = 0;
                y += 1;
            }

            cell
        });
    }

    pub fn render(&mut self) -> io::Result<()> {
        let size = self.size;

        for (origin, element) in self.nav.render(size) {
            self.fill_rect(
                Rect::new(origin.x, origin.y, element.text.width() as u32, 1),
                element.background,
            );
            self.draw_text(
                &element.text,
                origin * (2, 1),
                Size::splat(0),
                element.foreground,
            );
        }

        self.painter.begin()?;

        for (previous, current) in self.cells.iter_mut() {
            if current == previous {
                continue;
            }

            previous.quadrant = current.quadrant;
            previous.grapheme = current.grapheme.clone();

            self.painter.paint(current)?;
        }

        self.painter.end(self.nav.cursor())?;

        Ok(())
    }

    /// Draw the background from a pixel array encoded in RGBA8888
    pub fn draw_background(&mut self, pixels: &[u8], pixels_size: Size, _rect: Rect) {
        let viewport = self.size.cast::<usize>();
        let pixels_size = pixels_size.cast::<usize>();

        if pixels_size.width == 0
            || pixels_size.height == 0
            || viewport.width == 0
            || viewport.height == 0
        {
            return;
        }

        let expected = pixels_size.width * pixels_size.height * 4;
        if pixels.len() < expected {
            log::debug!(
                "unexpected size, actual: {}, expected: {}",
                pixels.len(),
                expected
            );
            return;
        }

        let image = match Self::bgra_frame_to_image(pixels, pixels_size) {
            Some(image) => image,
            None => {
                log::error!(
                    "failed to convert framebuffer into an RGBA image ({}x{})",
                    pixels_size.width,
                    pixels_size.height
                );
                return;
            }
        };
        let options = RenderOptions::new()
            .width(viewport.width as u32)
            .height(viewport.height as u32)
            .colored(true)
            .invert(true)
            .charset(charsets::DEFAULT);
        let mut frame = String::new();

        if let Err(error) = render_image_to(&image, &mut frame, &options) {
            log::error!("failed to render framebuffer with rascii: {error}");
            return;
        }

        self.fill_rect(
            Rect::new(0, 1, self.size.width, self.size.height),
            Color::black(),
        );
        self.draw_rascii_frame(&frame);
    }

    pub fn clear_text(&mut self) {
        for (_, cell) in self.cells.iter_mut() {
            cell.grapheme = None
        }
    }

    pub fn set_title(&self, title: &str) -> io::Result<()> {
        let mut stdout = io::stdout();

        write!(stdout, "\x1b]0;{title}\x07")?;
        write!(stdout, "\x1b]1;{title}\x07")?;
        write!(stdout, "\x1b]2;{title}\x07")?;

        stdout.flush()
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.draw(rect, |cell| {
            cell.grapheme = None;
            cell.quadrant = (color, color, color, color);
        })
    }

    pub fn draw<F>(&mut self, bounds: Rect, mut draw: F)
    where
        F: FnMut(&mut Cell),
    {
        let origin = bounds.origin.cast::<usize>();
        let size = bounds.size.cast::<usize>();
        let viewport_width = self.size.width as usize;
        let top = origin.y;
        let bottom = top + size.height;

        // Iterate over each row
        for y in top..bottom {
            let left = y * viewport_width + origin.x;
            let right = left + size.width;

            for (_, current) in self.cells[left..right].iter_mut() {
                draw(current)
            }
        }
    }

    /// Render some text into the terminal output
    pub fn draw_text(&mut self, string: &str, origin: Point, size: Size, color: Color) {
        // Get an iterator starting at the text origin
        let len = self.cells.len();
        let viewport = &self.size.cast::<usize>();

        if size.width > 2 && size.height > 2 {
            let origin = (origin.cast::<f32>() / (2.0, 4.0) + (0.0, 1.0)).round();
            let size = (size.cast::<f32>() / (2.0, 4.0)).round();
            let left = (origin.x.max(0.0) as usize).min(viewport.width);
            let right = ((origin.x + size.width).max(0.0) as usize).min(viewport.width);
            let top = (origin.y.max(0.0) as usize).min(viewport.height);
            let bottom = ((origin.y + size.height).max(0.0) as usize).min(viewport.height);

            for y in top..bottom {
                let index = y * viewport.width;
                let start = index + left;
                let end = index + right;

                for (_, cell) in self.cells[start..end].iter_mut() {
                    cell.grapheme = None
                }
            }
        } else {
            // Compute the buffer index based on the position
            let index = origin.x / 2 + (origin.y + 1) / 4 * (viewport.width as i32);
            let mut iter = self.cells[len.min(index as usize)..].iter_mut();

            // Get every Unicode grapheme in the input string
            for grapheme in UnicodeSegmentation::graphemes(string, true) {
                let width = grapheme.width();

                for index in 0..width {
                    // Get the next terminal cell at the given position
                    match iter.next() {
                        // Stop if we're at the end of the buffer
                        None => return,
                        // Set the cell to the current grapheme
                        Some((_, cell)) => {
                            let next = Grapheme {
                                // Create a new shared reference to the text
                                color,
                                index,
                                width,
                                // Export the set of unicode code points for this graphene into an UTF-8 string
                                char: grapheme.to_string(),
                            };

                            if match cell.grapheme {
                                None => true,
                                Some(ref previous) => {
                                    previous.color != next.color || previous.char != next.char
                                }
                            } {
                                cell.grapheme = Some(Rc::new(next))
                            }
                        }
                    }
                }
            }
        }
    }

    fn bgra_frame_to_image(pixels: &[u8], size: Size<usize>) -> Option<DynamicImage> {
        let mut rgba = Vec::with_capacity(size.width * size.height * 4);

        for chunk in pixels.chunks_exact(4).take(size.width * size.height) {
            rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], chunk[3]]);
        }

        let image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_vec(
            size.width as u32,
            size.height as u32,
            rgba,
        )?;

        Some(DynamicImage::ImageRgba8(image))
    }

    fn draw_rascii_frame(&mut self, frame: &str) {
        let viewport = self.size.cast::<usize>();
        let black = Color::black();
        let mut color = black;
        let mut row = 0usize;
        let mut col = 0usize;
        let bytes = frame.as_bytes();
        let mut index = 0usize;

        while index < bytes.len() && row < viewport.height {
            match bytes[index] {
                b'\x1b' => {
                    index = self.parse_rascii_escape(bytes, index, &mut color);
                }
                b'\n' => {
                    row += 1;
                    col = 0;
                    index += 1;
                }
                _ => {
                    let Ok(remaining) = std::str::from_utf8(&bytes[index..]) else {
                        break;
                    };
                    let Some(ch) = remaining.chars().next() else {
                        break;
                    };
                    let width = UnicodeWidthChar::width(ch).unwrap_or(1).max(1);

                    if col < viewport.width {
                        self.write_browser_char(row, col, ch, color);
                    }

                    col = (col + width).min(viewport.width);
                    index += ch.len_utf8();
                }
            }
        }
    }

    fn parse_rascii_escape(&self, bytes: &[u8], start: usize, color: &mut Color) -> usize {
        let Some(b'[') = bytes.get(start + 1).copied() else {
            return start + 1;
        };
        let mut end = start + 2;

        while end < bytes.len() && bytes[end] != b'm' {
            end += 1;
        }

        if end >= bytes.len() {
            return bytes.len();
        }

        if let Ok(params) = std::str::from_utf8(&bytes[start + 2..end]) {
            self.apply_sgr(params, color);
        }

        end + 1
    }

    fn apply_sgr(&self, params: &str, color: &mut Color) {
        if params.is_empty() {
            return;
        }

        let values = params
            .split(';')
            .filter_map(|value| value.parse::<u16>().ok())
            .collect::<Vec<_>>();
        let mut index = 0usize;

        while index < values.len() {
            match values[index] {
                0 => *color = Color::black(),
                38 if index + 4 < values.len() && values[index + 1] == 2 => {
                    *color = Color::new(
                        values[index + 2] as u8,
                        values[index + 3] as u8,
                        values[index + 4] as u8,
                    );
                    index += 4;
                }
                _ => {}
            }

            index += 1;
        }
    }

    fn write_browser_char(&mut self, row: usize, col: usize, ch: char, color: Color) {
        let viewport = self.size.cast::<usize>();
        let width = UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
        let text = ch.to_string();
        let foreground = Self::contrast_color(color);

        for char_index in 0..width {
            let column = col + char_index;
            if column >= viewport.width {
                break;
            }

            let cell_index = (row + 1) * viewport.width + column;
            let (_, cell) = &mut self.cells[cell_index];
            cell.quadrant = (color, color, color, color);
            cell.grapheme = if ch == ' ' {
                None
            } else {
                Some(Rc::new(Grapheme {
                    char: text.clone(),
                    index: char_index,
                    width,
                    color: foreground,
                }))
            };
        }
    }

    fn contrast_color(background: Color) -> Color {
        let luma =
            0.299 * background.r as f32 + 0.587 * background.g as f32 + 0.114 * background.b as f32;

        if luma >= 140.0 {
            Color::black()
        } else {
            Color::new(255, 255, 255)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(r: u8, g: u8, b: u8) -> [u8; 4] {
        [r, g, b, 255]
    }

    #[test]
    fn draw_background_renders_rascii_cells_into_browser_viewport() {
        let mut renderer = Renderer::new();
        renderer.set_size(Size::new(2, 1));

        let mut pixels = Vec::new();
        for color in [rgba(255, 0, 0), rgba(0, 0, 255)] {
            pixels.extend_from_slice(&color);
        }

        renderer.draw_background(&pixels, Size::new(2, 1), Rect::new(0, 0, 2, 1));

        let left = &renderer.cells[2].1;
        let right = &renderer.cells[3].1;
        let red = Color::new(255, 0, 0);
        let blue = Color::new(0, 0, 255);

        assert_eq!(left.quadrant, (red, red, red, red));
        assert_eq!(right.quadrant, (blue, blue, blue, blue));

        if let Some(grapheme) = &left.grapheme {
            assert_eq!(grapheme.color, Color::new(255, 255, 255));
        }

        if let Some(grapheme) = &right.grapheme {
            assert_eq!(grapheme.color, Color::new(255, 255, 255));
        }
    }
}
