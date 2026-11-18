use crate::{
    Browser, EvaluationResult, ViewId,
    commands::{ACTIONS, Action},
};
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use std::cell::RefCell;

// Font discovery and glyph rasterization are shared by palettes on this window thread.
thread_local! {
    static FONTS: RefCell<(FontSystem, SwashCache)> = RefCell::new((FontSystem::new(), SwashCache::new()));
}

#[derive(Default)]
pub(super) struct Palette {
    query: String,
    selected: usize,
    prompt: Option<Action>,
    evaluation: Option<u64>,
    output: Option<String>,
}
pub(super) enum Result {
    Keep,
    Close,
    Quit,
}

impl Palette {
    pub fn query(&self) -> &str {
        self.output.as_deref().unwrap_or(&self.query)
    }
    fn matches(&self) -> Vec<(Action, &'static str)> {
        let query = self.query.to_lowercase();
        ACTIONS
            .iter()
            .filter(|entry| {
                entry.label.to_lowercase().contains(&query) || entry.name.contains(&query)
            })
            .map(|entry| (entry.action, entry.label))
            .collect()
    }
    pub fn text(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.query.len() < 8192 {
                self.query.push(c);
            }
        }
        self.selected = 0;
    }
    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }
    pub fn clear(&mut self) {
        self.output = None;
        self.evaluation = None;
        self.query.clear();
        self.selected = 0;
    }
    pub fn select(&mut self, delta: isize) {
        let count = self.matches().len();
        if count > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
        }
    }
    pub fn escape(&mut self) -> Result {
        if self.output.take().is_some() {
            self.evaluation = None;
            self.clear();
            Result::Keep
        } else if self.prompt.take().is_some() {
            self.clear();
            Result::Keep
        } else {
            Result::Close
        }
    }
    pub fn submit(
        &mut self,
        browser: &mut Browser,
        view: ViewId,
    ) -> std::result::Result<Result, String> {
        if self.output.is_some() {
            return Ok(self.escape());
        }
        if let Some(Action::Eval) = self.prompt {
            self.evaluation = Some(browser.eval(view, &self.query)?);
            self.output = Some("Evaluating…".into());
            self.prompt = None;
            return Ok(Result::Keep);
        }
        if let Some(action) = self.prompt {
            let url = self.query.trim();
            if url.is_empty() {
                return Ok(Result::Keep);
            }
            if action == Action::Open {
                browser.open(url)?;
            } else {
                browser.navigate(view, url)?;
            }
            return Ok(Result::Close);
        }
        let Some((action, _)) = self.matches().get(self.selected).copied() else {
            return Ok(Result::Keep);
        };
        match action {
            Action::Navigate | Action::Open | Action::Eval => {
                self.prompt = Some(action);
                self.clear();
                return Ok(Result::Keep);
            }
            Action::Back => browser.back(view)?,
            Action::Forward => browser.forward(view)?,
            Action::Reload => browser.reload(view)?,
            Action::Stop => browser.stop(view)?,
            Action::Close => browser.close(view)?,
            Action::ZoomIn | Action::ZoomOut | Action::ResetZoom => {
                action.adjust_zoom(browser, view)?
            }
            Action::Views => {
                self.output = Some(
                    browser
                        .views()
                        .map(|v| format!("{}  {}  {}", v.id, v.title, v.url))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                return Ok(Result::Keep);
            }
            Action::Quit => return Ok(Result::Quit),
        }
        Ok(Result::Close)
    }
    pub fn evaluated(&mut self, request: u64, result: &EvaluationResult) -> bool {
        if self.evaluation != Some(request) {
            return false;
        }
        self.evaluation = None;
        self.output = Some(match result {
            EvaluationResult::Json { value } => value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned),
            EvaluationResult::Undefined => "undefined".into(),
            EvaluationResult::Number { value } | EvaluationResult::Bigint { value } => {
                value.clone()
            }
            EvaluationResult::Error { name, message, .. } => format!("{name}: {message}"),
        });
        true
    }
    pub fn click(&mut self, x: i32, y: i32, width: u32, height: u32, scale: f64) -> bool {
        if self.prompt.is_some() || self.output.is_some() {
            return false;
        }
        let layout = Layout::new(width, height, scale);
        if x < layout.x as i32
            || x >= (layout.x + layout.width) as i32
            || y < (layout.y + 50 * layout.unit) as i32
        {
            return false;
        }
        let row = (y as usize - layout.y - 50 * layout.unit) / (16 * layout.unit);
        let visible = layout.visible_rows().min(self.matches().len());
        if row >= visible {
            return false;
        }
        self.selected = self.first_row(visible) + row;
        true
    }
    fn first_row(&self, visible: usize) -> usize {
        self.selected.saturating_sub(visible.saturating_sub(1))
    }
    pub fn draw(&self, pixels: &mut [u32], width: u32, height: u32, scale: f64) {
        let l = Layout::new(width, height, scale);
        let mut canvas = Canvas {
            pixels,
            width: width as usize,
            height: height as usize,
        };
        // Host-owned pixels: page scripts cannot inspect or alter the palette.
        canvas.rect(l.x, l.y, l.width, l.height, 0x18212b);
        canvas.rect(l.x, l.y, l.width, 2 * l.unit, 0x88cfb0);
        let title = match self.prompt {
            Some(Action::Eval) => "EVALUATE - enter JavaScript",
            Some(Action::Open) => "NEW WINDOW - enter URL",
            Some(_) => "NAVIGATE - enter URL",
            None if self.output.is_some() => "RESULT - copy or Esc to return",
            None => "COMMANDS - type to filter",
        };
        canvas.text(
            l.x + 10 * l.unit,
            l.y + 10 * l.unit,
            title,
            l.unit,
            0x88cfb0,
            l.x + l.width,
        );
        if let Some(output) = &self.output {
            for (i, line) in output.lines().take(l.visible_rows() + 1).enumerate() {
                canvas.text(
                    l.x + 10 * l.unit,
                    l.y + (30 + 16 * i) * l.unit,
                    line,
                    l.unit,
                    0xf3f6f8,
                    l.x + l.width,
                );
            }
            return;
        }
        let columns = (l.width.saturating_sub(28 * l.unit) / (8 * l.unit)).max(1);
        let tail: String = self
            .query
            .chars()
            .rev()
            .take(columns)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        canvas.text(
            l.x + 10 * l.unit,
            l.y + 30 * l.unit,
            &format!("> {tail}_"),
            l.unit,
            0xf3f6f8,
            l.x + l.width,
        );
        if self.prompt.is_none() {
            let matches = self.matches();
            let visible = l.visible_rows().min(matches.len());
            if matches.is_empty() {
                canvas.text(
                    l.x + 10 * l.unit,
                    l.y + 50 * l.unit,
                    "No matching commands",
                    l.unit,
                    0x9caab8,
                    l.x + l.width,
                );
            }
            for (row, (index, (_, label))) in matches
                .iter()
                .enumerate()
                .skip(self.first_row(visible))
                .take(visible)
                .enumerate()
            {
                let y = l.y + (50 + row * 16) * l.unit;
                if index == self.selected {
                    canvas.rect(
                        l.x + 4 * l.unit,
                        y - 3 * l.unit,
                        l.width.saturating_sub(8 * l.unit),
                        15 * l.unit,
                        0x36584d,
                    );
                }
                canvas.text(l.x + 10 * l.unit, y, label, l.unit, 0xf3f6f8, l.x + l.width);
            }
        }
        canvas.text(
            l.x + 10 * l.unit,
            l.y + l.height.saturating_sub(15 * l.unit),
            "Enter: run   Esc: back/close",
            l.unit,
            0x9caab8,
            l.x + l.width,
        );
    }
}
struct Layout {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    unit: usize,
}
impl Layout {
    fn new(width: u32, height: u32, scale: f64) -> Self {
        let unit = (scale * 2.).round().clamp(1., 8.) as usize;
        let w = (320 * unit).min((width as usize).saturating_sub(16));
        let h = (250 * unit).min((height as usize).saturating_sub(32));
        Self {
            x: (width as usize - w) / 2,
            y: 16.min(height as usize),
            width: w,
            height: h,
            unit,
        }
    }
    fn visible_rows(&self) -> usize {
        (self.height / self.unit).saturating_sub(75) / 16
    }
}
struct Canvas<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
}
impl Canvas<'_> {
    fn rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: u32) {
        for row in y.min(self.height)..y.saturating_add(height).min(self.height) {
            let start = row * self.width + x.min(self.width);
            let end = row * self.width + x.saturating_add(width).min(self.width);
            self.pixels[start..end].fill(color);
        }
    }
    fn text(&mut self, x: usize, y: usize, text: &str, unit: usize, color: u32, right: usize) {
        let width = right.min(self.width).saturating_sub(x);
        if width == 0 || y >= self.height {
            return;
        }
        FONTS.with_borrow_mut(|(fonts, cache)| {
            let mut line = Buffer::new_empty(Metrics::new(11.0 * unit as f32, 14.0 * unit as f32));
            line.set_size(Some(width as f32), None);
            line.set_wrap(Wrap::None);
            line.set_text(
                text,
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                None,
            );
            line.draw(
                fonts,
                cache,
                Color(color | 0xff000000),
                |dx, dy, w, h, ink| {
                    let left = x as i32 + dx;
                    let top = y as i32 + dy;
                    let alpha = u32::from(ink.a());
                    for row in top.max(0)..(top + h as i32).min(self.height as i32) {
                        for col in left.max(x as i32)..(left + w as i32).min((x + width) as i32) {
                            let pixel = &mut self.pixels[row as usize * self.width + col as usize];
                            let blend = |shift: u32| {
                                ((((ink.0 >> shift) & 255) * alpha
                                    + ((*pixel >> shift) & 255) * (255 - alpha))
                                    / 255)
                                    << shift
                            };
                            *pixel = blend(16) | blend(8) | blend(0);
                        }
                    }
                },
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_selects_and_clips_on_tiny_surfaces() {
        let mut palette = Palette::default();
        palette.text("zoom");
        assert_eq!(palette.matches().len(), 3);
        palette.select(-1);
        assert_eq!(palette.matches()[palette.selected].0, Action::ResetZoom);
        let mut pixels = vec![0; 12];
        palette.draw(&mut pixels, 3, 4, 2.);
        palette.clear();
        palette.text("日x");
        palette.backspace();
        assert_eq!(palette.query, "日");
    }
}
