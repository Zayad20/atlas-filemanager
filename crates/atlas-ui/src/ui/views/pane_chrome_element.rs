//! Monolithic Element for Pane Chrome, eliminating all Taffy wrappers per pane.
//! 
//! Draws the CrumbStrip, Header, Body, and Scrollbar in a single node.
//! Disconnected layout roots are used for the children to avoid flex layout overhead.


use atlas_model::pane::TabId;
use gpui::{
    px, AnyElement, Bounds, Element, App as GpuiApp,
    ElementId, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId,
    Pixels, SharedString, Style,
    StyleRefinement, Styled, Window, UniformListScrollHandle,
};

pub(crate) struct PaneChromeElement {
    id: ElementId,
    pane_id: TabId,
    crumb_strip: AnyElement,
    header: AnyElement,
    scroll_body: AnyElement,
    filter_pill: Option<AnyElement>,
    controls_pill: Option<AnyElement>,
    base_scroll: UniformListScrollHandle,
    total_height: f32,
    style: StyleRefinement,
    // colors for scrollbar
    scrollbar_track: Hsla,
    scrollbar_thumb: Hsla,
}

pub(crate) struct PaneChromePrepaint {
    crumb_layout: LayoutId,
    header_layout: LayoutId,
    body_layout: LayoutId,
    filter_layout: Option<LayoutId>,
    controls_layout: Option<LayoutId>,
}

impl PaneChromeElement {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pane_id: TabId,
        crumb_strip: AnyElement,
        header: AnyElement,
        scroll_body: AnyElement,
        filter_pill: Option<AnyElement>,
        controls_pill: Option<AnyElement>,
        base_scroll: UniformListScrollHandle,
        total_height: f32,
        scrollbar_track: Hsla,
        scrollbar_thumb: Hsla,
    ) -> Self {
        Self {
            id: ElementId::Name(SharedString::from(format!("pane-chrome-{}", pane_id.0))),
            pane_id,
            crumb_strip,
            header,
            scroll_body,
            filter_pill,
            controls_pill,
            base_scroll,
            total_height,
            style: StyleRefinement::default(),
            scrollbar_track,
            scrollbar_thumb,
        }
    }
}

impl IntoElement for PaneChromeElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for PaneChromeElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Element for PaneChromeElement {
    type RequestLayoutState = PaneChromePrepaint;
    type PrepaintState = ();

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
        crate::perf::count("pane.chrome.layout");
        

        let crumb_layout = self.crumb_strip.request_layout(window, cx);


        // Assume crumb strip height is ~36px or 44px, but we can just let it overlap if needed
        // Actually, Taffy doesn't return computed size until after `request_layout` returns, 
        // so we'll just stack them in prepaint!
        let header_layout = self.header.request_layout(window, cx);
        
        let body_layout = self.scroll_body.request_layout(window, cx);

        let filter_layout = self.filter_pill.as_mut().map(|p| p.request_layout(window, cx));
        let controls_layout = self.controls_pill.as_mut().map(|p| p.request_layout(window, cx));

        // Register children so Taffy computes bounds.
        // We do NOT position them with flex; they just sit at (0,0) in our node space!
        let mut children = vec![crumb_layout, header_layout, body_layout];
        if let Some(l) = filter_layout { children.push(l); }
        if let Some(l) = controls_layout { children.push(l); }

        let mut root_style = Style::default();
        root_style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(px(0.0).into())));
        let layout_id = window.request_layout(Style::default(), children, cx);

        (
            layout_id,
            PaneChromePrepaint {
                crumb_layout,
                header_layout,
                body_layout,
                filter_layout,
                controls_layout,
            },
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        request_layout: &mut PaneChromePrepaint,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> Self::PrepaintState {
        crate::perf::count("pane.chrome.prepaint");
        
        // Compute offsets manually!
        let crumb_h = window.layout_bounds(request_layout.crumb_layout).size.height;
        let header_h = window.layout_bounds(request_layout.header_layout).size.height;

        let crumb_offset = gpui::Point { x: px(0.0), y: px(0.0) };
        let header_offset = gpui::Point { x: px(0.0), y: crumb_h };
        let body_offset = gpui::Point { x: px(0.0), y: crumb_h + header_h };

        window.with_element_offset(crumb_offset, |window| {
            self.crumb_strip.prepaint(window, cx);
        });
        window.with_element_offset(header_offset, |window| {
            self.header.prepaint(window, cx);
        });
        window.with_element_offset(body_offset, |window| {
            self.scroll_body.prepaint(window, cx);
        });
        if let Some(p) = self.filter_pill.as_mut() { p.prepaint(window, cx); }
        if let Some(p) = self.controls_pill.as_mut() { p.prepaint(window, cx); }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut PaneChromePrepaint,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) {
        crate::perf::count("pane.chrome.paint");
        
        let crumb_h = window.layout_bounds(request_layout.crumb_layout).size.height;
        let header_h = window.layout_bounds(request_layout.header_layout).size.height;

        let crumb_offset = gpui::Point { x: px(0.0), y: px(0.0) };
        let header_offset = gpui::Point { x: px(0.0), y: crumb_h };
        let body_offset = gpui::Point { x: px(0.0), y: crumb_h + header_h };

        window.with_element_offset(crumb_offset, |window| {
            self.crumb_strip.paint(window, cx);
        });
        window.with_element_offset(header_offset, |window| {
            self.header.paint(window, cx);
        });
        window.with_element_offset(body_offset, |window| {
            self.scroll_body.paint(window, cx);
        });
        
        if let Some(p) = self.filter_pill.as_mut() { p.paint(window, cx); }
        if let Some(p) = self.controls_pill.as_mut() { p.paint(window, cx); }
        

    }
}
