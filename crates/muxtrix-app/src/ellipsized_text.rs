use std::borrow::Cow;

use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::text::{self, Renderer as _, paragraph};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Layout, Widget};
use iced::{Color, Element, Fill, Font, Length, Pixels, Rectangle, Size, Theme};

/// Single-line native text that keeps its trailing ellipsis inside the width
/// assigned by the surrounding layout.
///
/// Iced clips an overflowing `Wrapping::None` paragraph, but it does not add an
/// ellipsis. Estimating a cutoff from character count is not reliable for the
/// configurable proportional interface fonts Muxtrix supports. This widget
/// shapes the text with the active renderer after the trailing row content has
/// claimed its space, then caches the fitting prefix until any input changes.
pub(super) struct EllipsizedText<'a> {
    content: Cow<'a, str>,
    size: Pixels,
    font: Font,
    color: Color,
    width: Length,
}

impl<'a> EllipsizedText<'a> {
    pub(super) fn owned(
        content: String,
        size: impl Into<Pixels>,
        font: Font,
        color: Color,
    ) -> Self {
        Self {
            content: Cow::Owned(content),
            size: size.into(),
            font,
            color,
            width: Fill,
        }
    }

    /// Hug the fitted copy instead of claiming the whole lane.
    ///
    /// A row whose trailing marks belong to the copy — the footer's account
    /// dot rather than a trailing column of its own — wants the mark to follow
    /// the name. The bound then has to come from a caller that caps this
    /// widget's width, which is what decides where the ellipsis lands.
    pub(super) fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }
}

struct State {
    paragraph: paragraph::Plain<<iced::Renderer as text::Renderer>::Paragraph>,
    source: String,
    bounds: Size,
    size: Pixels,
    font: Font,
}

impl State {
    fn new() -> Self {
        Self {
            paragraph: paragraph::Plain::default(),
            source: String::new(),
            bounds: Size::ZERO,
            size: Pixels::ZERO,
            font: Font::DEFAULT,
        }
    }

    fn update(&mut self, source: &str, bounds: Size, size: Pixels, font: Font) {
        if self.source == source && self.bounds == bounds && self.size == size && self.font == font
        {
            return;
        }

        let rendered = ellipsize_to_width(source, |candidate| {
            let _ = self.paragraph.update(text::Text {
                content: candidate,
                bounds,
                size,
                line_height: text::LineHeight::default(),
                font,
                align_x: text::Alignment::Default,
                align_y: iced::alignment::Vertical::Top,
                shaping: text::Shaping::default(),
                wrapping: text::Wrapping::None,
            });
            self.paragraph.min_width() <= bounds.width
        });

        // The fitter's last probe is not guaranteed to be the winning prefix.
        // Leave the cached paragraph holding exactly what draw will render.
        let _ = self.paragraph.update(text::Text {
            content: &rendered,
            bounds,
            size,
            line_height: text::LineHeight::default(),
            font,
            align_x: text::Alignment::Default,
            align_y: iced::alignment::Vertical::Top,
            shaping: text::Shaping::default(),
            wrapping: text::Wrapping::None,
        });
        self.source = source.to_owned();
        self.bounds = bounds;
        self.size = size;
        self.font = font;
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for EllipsizedText<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::sized(limits, self.width, Length::Shrink, |limits| {
            let bounds = limits.max();
            let state = tree.state.downcast_mut::<State>();
            state.update(self.content.as_ref(), bounds, self.size, self.font);
            state.paragraph.min_bounds()
        })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor: iced::advanced::mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        renderer.fill_paragraph(
            state.paragraph.raw(),
            layout.bounds().position(),
            self.color,
            *viewport,
        );
    }

    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        // Accessibility and text inspection receive the complete identity,
        // even when visual chrome needs to shorten it.
        operation.text(None, layout.bounds(), self.content.as_ref());
    }
}

impl<'a, Message> From<EllipsizedText<'a>> for Element<'a, Message>
where
    Message: 'a,
{
    fn from(text: EllipsizedText<'a>) -> Self {
        Element::new(text)
    }
}

fn ellipsize_to_width(value: &str, mut fits: impl FnMut(&str) -> bool) -> String {
    if fits(value) {
        return value.to_owned();
    }

    let boundaries: Vec<_> = value.char_indices().map(|(index, _)| index).collect();
    let mut low = 0;
    let mut high = boundaries.len();

    while low < high {
        let middle = (low + high).div_ceil(2);
        let end = boundaries.get(middle).copied().unwrap_or(value.len());
        let candidate = format!("{}…", &value[..end]);
        if fits(&candidate) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    let end = boundaries.get(low).copied().unwrap_or(value.len());
    format!("{}…", &value[..end])
}

#[cfg(test)]
mod tests {
    use super::ellipsize_to_width;

    #[test]
    fn fitter_keeps_the_ellipsis_inside_variable_width_copy() {
        let width = |value: &str| {
            value
                .chars()
                .map(|character| match character {
                    'W' => 3,
                    '…' => 2,
                    _ => 1,
                })
                .sum::<usize>()
        };

        assert_eq!(
            ellipsize_to_width("WWWW-agent", |candidate| width(candidate) <= 10),
            "WW…"
        );
        assert_eq!(
            ellipsize_to_width("short", |candidate| width(candidate) <= 10),
            "short"
        );
    }
}
