//! Custom GPUI `Element` for the entire Sidebar body.
//!
//! Replaces hundreds of Taffy nodes (one per row/section) with a single leaf element.
//! Collects hit-zones in `prepaint` and routes interactions in `paint`.

use crate::app::{App, SidebarHit, SidebarHitModel};
use crate::ui::icons::AtlasIcon;
use crate::ui::sidebar::{SectionKind, SidebarStyle};
use atlas_model::location::Location;
use gpui::{
    point, px, size, App as GpuiApp, Bounds, ContentMask, DispatchPhase, Element, ElementId, Font,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Refineable, SharedString, ShapedLine, Style,
    StyleRefinement, Styled, TextAlign, WeakEntity, Window,
};

pub(crate) struct SidebarElement {
    id: ElementId,
    app: WeakEntity<App>,
    style: SidebarStyle,
    current_path: Option<std::path::PathBuf>,
    current_location: Option<Location>,
    font: Font,
    font_size: Pixels,
    element_style: StyleRefinement,
}

pub(crate) struct SidebarPrepaint {
    // Optional caching
}

impl SidebarElement {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        app: WeakEntity<App>,
        style: SidebarStyle,
        current_path: Option<std::path::PathBuf>,
        current_location: Option<Location>,
        font: Font,
        font_size: Pixels,
    ) -> Self {
        Self {
            id: ElementId::Name(SharedString::from("sidebar-body-element")),
            app,
            style,
            current_path,
            current_location,
            font,
            font_size,
            element_style: StyleRefinement::default(),
        }
    }
}

impl IntoElement for SidebarElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for SidebarElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.element_style
    }
}

impl Element for SidebarElement {
    type RequestLayoutState = Style;
    type PrepaintState = SidebarPrepaint;

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
        crate::perf::count("sidebar.element.layout");
        let mut style = Style::default();
        style.refine(&self.element_style);
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
        crate::perf::count("sidebar.element.prepaint");
        
        let mut hit_model = SidebarHitModel {
            bounds,
            zones: Vec::new(),
        };

        if let Some(app) = self.app.upgrade() {
            let app_ref = app.read(cx);
            let sections = &app_ref.state.profiles.active().sidebar_sections;
            
            let row_h = if self.style.windows_theme { 30.0 } else { 24.0 };
            let mut y = bounds.top();
            
            // Re-implement the section logic here to build the hit map.
            // For now, we just map out the bounds.
            for section in sections {
                if section.hidden { continue; }
                
                // Section Header (32px high?)
                let header_h = px(24.0);
                let header_bounds = Bounds::new(point(bounds.left(), y), size(bounds.size.width, header_h));
                // hit_model.zones.push((header_bounds, SidebarHit::SectionHeader(0)));
                y += header_h;
                
                // Body rows...
                match section.kind {
                    SectionKind::Places => {
                        for i in 0..8 {
                            let row_bounds = Bounds::new(point(bounds.left(), y), size(bounds.size.width, px(row_h)));
                            // hit_model.zones.push((row_bounds, SidebarHit::NavRow(...)));
                            y += px(row_h);
                        }
                    }
                    _ => {}
                }
            }
            
            // Save the hit model back to the App state so that `sidebar.rs` interactions can use it.
            // But prepaint cannot mutate `App` easily without `update` which requires a mutable context.
            // Wait, we CAN update `App` in prepaint by deferring it!
            // `window.defer(move |cx| { app.update(cx, |app, cx| { app.sidebar_hit_model = Some(hit_model); }) })`
        }

        SidebarPrepaint {}
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
        crate::perf::count("sidebar.element.paint");
        
        // Paint the sidebar manually...
        window.paint_quad(gpui::fill(gpui::red()).bounds(bounds)); // TEST
    }
}
