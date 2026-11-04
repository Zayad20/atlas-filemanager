//! Build size-dependent content after its current container bounds are known.
//!
//! Like GPUI's virtual lists, the child is laid out during prepaint. Previous
//! frame measurements remain useful for input, but never determine this layout.
use gpui::prelude::*;
use gpui::{
    div, relative, size, AnyElement, App, AvailableSpace, Bounds, ContentMask, Element,
    GlobalElementId, InspectorElementId, LayoutId, Pixels, Style, Window,
};

pub(crate) fn current_bounds(
    build: impl FnMut(Bounds<Pixels>, &mut Window, &mut App) -> AnyElement + 'static,
) -> impl IntoElement {
    CurrentBounds { build }
}

struct CurrentBounds<F> {
    build: F,
}

impl<F: 'static> IntoElement for CurrentBounds<F>
where
    F: FnMut(Bounds<Pixels>, &mut Window, &mut App) -> AnyElement,
{
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<F: 'static> Element for CurrentBounds<F>
where
    F: FnMut(Bounds<Pixels>, &mut Window, &mut App) -> AnyElement,
{
    type RequestLayoutState = ();
    type PrepaintState = AnyElement;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        // Finish any entity update in build before requesting child layout:
        // retained child views may update the same App when they render.
        let content = (self.build)(bounds, window, cx);
        let mut child = div()
            .relative()
            .w(bounds.size.width)
            .h(bounds.size.height)
            .child(content)
            .into_any_element();
        child.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(bounds.size.height),
            ),
            window,
            cx,
        );
        window.with_content_mask(
            Some(ContentMask {
                bounds,
                ..Default::default()
            }),
            |window| {
                child.prepaint_at(bounds.origin, window, cx);
            },
        );
        child
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(
            Some(ContentMask {
                bounds,
                ..Default::default()
            }),
            |window| {
                child.paint(window, cx);
            },
        );
    }
}
