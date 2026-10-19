//! Optional native render/interaction harness. Never linked into normal builds.
use crate::app::App;
use gpui::{Entity, Window};
use std::path::PathBuf;

pub(crate) fn install(view: &Entity<App>, window: &mut Window, cx: &mut gpui::App) {
    let Some(directory) = std::env::var_os("ATLAS_CONVERSION_LAB").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Sample.png");
    image::RgbaImage::from_pixel(160, 100, image::Rgba([240, 130, 65, 255]))
        .save(&source)
        .unwrap();
    view.update(cx, |app, cx| {
        app.state.overlay.show_getting_started = false;
        app.open_converter_for(vec![source]);
        cx.notify();
    });
    schedule(view.clone(), directory, 0, window);
}

fn schedule(view: Entity<App>, directory: PathBuf, step: usize, window: &Window) {
    window.on_next_frame(move |window, cx| {
        if step == 12 {
            window.render_to_image().unwrap().save(directory.join("wheel-desktop.png")).unwrap();
            window.resize(gpui::size(gpui::px(900.), gpui::px(560.)));
        }
        if step == 25 {
            window.render_to_image().unwrap().save(directory.join("wheel-laptop.png")).unwrap();
            view.update(cx, |app, cx| { app.converter.as_mut().unwrap().expanded = true; cx.notify(); });
        }
        if step == 16 {
            window.dispatch_keystroke(gpui::Keystroke::parse("right").unwrap(), cx);
            assert_eq!(view.read(cx).converter.as_ref().unwrap().selected, 1);
            window.dispatch_keystroke(gpui::Keystroke::parse("left").unwrap(), cx);
            assert_eq!(view.read(cx).converter.as_ref().unwrap().selected, 0);
        }
        if step == 38 {
            window.render_to_image().unwrap().save(directory.join("options-laptop.png")).unwrap();
            if std::env::var_os("ATLAS_CONVERSION_LAB_MOUSE").is_some() {
                let bounds = view.read(cx).converter.as_ref().unwrap().wheel_bounds.unwrap();
                let center = bounds.center();
                window.dispatch_event(gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    position: gpui::point(center.x, center.y - bounds.size.height * 0.34),
                    button: gpui::MouseButton::Left,
                    modifiers: gpui::Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }), cx);
            } else {
                window.dispatch_keystroke(gpui::Keystroke::parse("enter").unwrap(), cx);
            }
            assert!(matches!(view.read(cx).converter.as_ref().unwrap().phase, super::Phase::Running { .. }));
        }
        if step >= 50 && view.read(cx).converter.as_ref().is_some_and(|s| matches!(s.phase, super::Phase::Finished(_))) {
            window.render_to_image().unwrap().save(directory.join("results-laptop.png")).unwrap();
            let session = view.read(cx).converter.as_ref().unwrap();
            if let super::Phase::Finished(report) = &session.phase {
                assert!(report.errors().is_empty(), "{:?}", report.errors());
                assert_eq!(report.outputs().len(), 1);
                assert!(image::open(&report.outputs()[0]).is_ok());
                std::fs::write(directory.join("passed.txt"), "Native window rendered at 1280x800 and 900x560. JPEG conversion decoded successfully.\n").unwrap();
            }
            cx.quit(); return;
        }
        if step >= 600 { panic!("Conversion lab did not finish"); }
        window.refresh();
        schedule(view, directory, step + 1, window);
    });
}
