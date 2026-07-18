//! Single-node Element for the Sidebar Section Body, matching `DetailsBodyElement`.
//! 
//! Replaces the hundreds of Taffy nodes (one per row) with a single hit-tested canvas
//! per section. Handles tooltips, drag & drop, and context menus manually via `window.on_mouse_event`.

use crate::app::AtlasApp;
use crate::ui::sidebar::{SectionKind, SidebarStyle};
use atlas_model::location::Location;
use gpui::{
    point, px, size, App as GpuiApp, Bounds, ContentMask, DispatchPhase, Element, ElementId, Font,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Refineable, SharedString, ShapedLine, Style,
    StyleRefinement, Styled, TextAlign, WeakEntity, Window,
};
use std::sync::Arc;

pub(crate) struct SidebarSectionElement {
    id: ElementId,
    app: WeakEntity<AtlasApp>,
    kind: SectionKind,
    style: SidebarStyle,
    current_path: Option<std::path::PathBuf>,
    current_location: Option<Location>,
    font: Font,
    font_size: Pixels,
    element_style: StyleRefinement,
}

pub(crate) struct SidebarSectionPrepaint {
    // We will cache layout geometry here (rows, hit-boxes)
}

impl SidebarSectionElement {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        app: WeakEntity<AtlasApp>,
        kind: SectionKind,
        style: SidebarStyle,
        current_path: Option<std::path::PathBuf>,
        current_location: Option<Location>,
        font: Font,
        font_size: Pixels,
    ) -> Self {
        Self {
            id: ElementId::Name(SharedString::from(format!("sidebar-section-{}", kind as u8))),
            app,
            kind,
            style,
            current_path,
            current_location,
            font,
            font_size,
            element_style: StyleRefinement::default(),
        }
    }
}

impl IntoElement for SidebarSectionElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for SidebarSectionElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.element_style
    }
}

impl Element for SidebarSectionElement {
    type RequestLayoutState = Style;
    type PrepaintState = SidebarSectionPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> (LayoutId, Self::RequestLayoutState) {
        crate::perf::count("sidebar.section.layout");
        let mut style = Style::default();
        style.refine(&self.element_style);
        
        // Compute height based on number of items in this section
        let count = match self.kind {
            SectionKind::Places => 8, // Approx
            SectionKind::Bookmarks => 0, // Need to read from app state
            _ => 0,
        };
        let row_h = if self.style.windows_theme { 30.0 } else { 24.0 };
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(px(count as f32 * row_h))));

        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> Self::PrepaintState {
        crate::perf::count("sidebar.section.prepaint");
        // TODO: Build hit map and text shaping for sidebar rows
        SidebarSectionPrepaint {}
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) {
        crate::perf::count("sidebar.section.paint");
        // TODO: Paint text, icons, and background hover states
        
        let app_weak = self.app.clone();
        window.on_mouse_event(move |ev: &MouseDownEvent, _phase, window, cx| {
            if ev.button == MouseButton::Left && bounds.contains(&ev.position) {
                // Determine which row was clicked
            }
        });
    }
}
