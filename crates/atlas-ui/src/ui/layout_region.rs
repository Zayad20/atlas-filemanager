//! Tag a subtree so GPUI attributes `request_layout` nodes to a named region
//! (`[gpui-layout] sidebar=… dock=…`). Active when `GPUI_FRAME_LOG` is set.

use gpui::{
    enter_layout_region, AnyElement, App, Bounds, Element, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Window,
};

/// Wrap `child` so every layout node it creates counts under `name`.
pub fn layout_region(name: &'static str, child: impl IntoElement) -> LayoutRegion {
    LayoutRegion {
        name,
        child: child.into_any_element(),
    }
}

pub struct LayoutRegion {
    name: &'static str,
    child: AnyElement,
}

impl Element for LayoutRegion {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let _region = enter_layout_region(self.name);
        let layout_id = self.child.request_layout(window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // Nodes are already attributed in request_layout; region only needed there.
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

impl IntoElement for LayoutRegion {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}
