use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{Element, Event, Length, Point, Rectangle, Size, Theme, Vector, touch};

/// Places a pane menu in Iced's overlay layer and dismisses it when a pointer
/// press is not handled by the menu itself.
pub(super) struct Popover<'a, Message> {
    content: Element<'a, Message>,
    popover: Option<Element<'a, Message>>,
    on_dismiss: Message,
}

impl<'a, Message> Popover<'a, Message> {
    pub(super) fn new(
        content: impl Into<Element<'a, Message>>,
        popover: Option<impl Into<Element<'a, Message>>>,
        on_dismiss: Message,
    ) -> Self {
        Self {
            content: content.into(),
            popover: popover.map(Into::into),
            on_dismiss,
        }
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for Popover<'_, Message>
where
    Message: Clone,
{
    fn children(&self) -> Vec<Tree> {
        std::iter::once(&self.content)
            .chain(self.popover.iter())
            .map(Tree::new)
            .collect()
    }

    fn diff(&self, tree: &mut Tree) {
        let children = std::iter::once(&self.content)
            .chain(self.popover.iter())
            .collect::<Vec<_>>();
        tree.diff_children_custom(
            &children,
            |tree, element| tree.diff((*element).as_widget()),
            |element| Tree::new((*element).as_widget()),
        );
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, iced::Renderer>> {
        if self.popover.is_none() {
            return self.content.as_widget_mut().overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            );
        }

        let popover = self.popover.as_mut()?;
        let state = tree.children.get_mut(1)?;

        Some(overlay::Element::new(Box::new(PaneMenuOverlay {
            popover,
            state,
            anchor: layout.bounds() + translation,
            viewport: *viewport,
            on_dismiss: self.on_dismiss.clone(),
        })))
    }
}

impl<'a, Message> From<Popover<'a, Message>> for Element<'a, Message>
where
    Message: Clone + 'a,
{
    fn from(popover: Popover<'a, Message>) -> Self {
        Element::new(popover)
    }
}

struct PaneMenuOverlay<'a, 'b, Message> {
    popover: &'a mut Element<'b, Message>,
    state: &'a mut Tree,
    anchor: Rectangle,
    viewport: Rectangle,
    on_dismiss: Message,
}

impl<Message> overlay::Overlay<Message, Theme, iced::Renderer> for PaneMenuOverlay<'_, '_, Message>
where
    Message: Clone,
{
    fn layout(&mut self, renderer: &iced::Renderer, bounds: Size) -> layout::Node {
        let limits = layout::Limits::new(Size::ZERO, bounds);
        let node = self
            .popover
            .as_widget_mut()
            .layout(self.state, renderer, &limits);
        let size = node.size();
        let viewport_right = self.viewport.x + self.viewport.width;
        let viewport_bottom = self.viewport.y + self.viewport.height;
        let x = (self.anchor.x + self.anchor.width - size.width - 6.0).clamp(
            self.viewport.x,
            (viewport_right - size.width).max(self.viewport.x),
        );
        let y = (self.anchor.y + 38.0).clamp(
            self.viewport.y,
            (viewport_bottom - size.height).max(self.viewport.y),
        );

        node.move_to(Point::new(x, y))
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.popover
            .as_widget_mut()
            .operate(self.state, layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.popover.as_widget_mut().update(
            self.state,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &self.viewport,
        );

        if shell.is_event_captured() {
            return;
        }

        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                | Event::Touch(touch::Event::FingerPressed { .. })
        ) {
            shell.publish(self.on_dismiss.clone());
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.popover.as_widget().mouse_interaction(
            self.state,
            layout,
            cursor,
            &self.viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.popover.as_widget().draw(
            self.state,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &self.viewport,
        );
    }

    fn overlay<'a>(
        &'a mut self,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
    ) -> Option<overlay::Element<'a, Message, Theme, iced::Renderer>> {
        self.popover.as_widget_mut().overlay(
            self.state,
            layout,
            renderer,
            &self.viewport,
            Vector::ZERO,
        )
    }
}
