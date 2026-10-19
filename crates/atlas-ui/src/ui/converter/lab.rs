//! Optional native render/interaction harness. Never linked into normal builds.
use crate::app::App;
use gpui::{Entity, Window};
use std::path::PathBuf;

pub(crate) fn install(view: &Entity<App>, window: &mut Window, cx: &mut gpui::App) {
    let Some(directory) = std::env::var_os("ATLAS_CONVERSION_LAB").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&directory).unwrap();
    let audio = std::env::var_os("ATLAS_CONVERSION_LAB_AUDIO").is_some();
    let source = directory.join(if audio { "Tone.wav" } else { "Sample.png" });
    if audio {
        // Explicit 16-bit PCM WAV fixture; keep the validation encoder unrelated
        // to the converter's audio implementation.
        let samples: Vec<_> = (0..22050)
            .flat_map(|i| {
                (((f64::from(i) * 440.0 * std::f64::consts::TAU / 44100.0).sin() * 16000.0).round()
                    as i16)
                    .to_le_bytes()
            })
            .collect();
        let mut wave = b"RIFF".to_vec();
        wave.extend_from_slice(&(36_u32 + samples.len() as u32).to_le_bytes());
        wave.extend_from_slice(
            b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x44\xac\0\0\x88\x58\x01\0\x02\0\x10\0data",
        );
        wave.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        wave.extend_from_slice(&samples);
        std::fs::write(&source, wave).unwrap();
    } else {
        image::RgbaImage::from_pixel(160, 100, image::Rgba([240, 130, 65, 255]))
            .save(&source)
            .unwrap();
    }
    view.update(cx, |app, cx| {
        app.state.overlay.show_getting_started = false;
        app.open_converter_for(vec![source]);
        cx.notify();
    });
    drive(view.clone(), directory, window, cx);
}

fn drive(view: Entity<App>, directory: PathBuf, window: &Window, cx: &mut gpui::App) {
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        for step in 0..=600 {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(20))
                .await;
            let done = handle
                .update(cx, |_, window, cx| {
                    // Exercise real layout/paint on a hidden window. No presenting
                    // or focusing a window on the user's desktop is necessary.
                    window.draw(cx).clear(cx);
                    frame(&view, &directory, step, window, cx)
                })
                .unwrap_or(true);
            if done {
                break;
            }
        }
    })
    .detach();
}

fn frame(
    view: &Entity<App>,
    directory: &std::path::Path,
    step: usize,
    window: &mut Window,
    cx: &mut gpui::App,
) -> bool {
    if step == 12 {
        if std::env::var_os("ATLAS_CONVERSION_LAB_AUDIO").is_some() {
            let session = view.read(cx).converter.as_ref().unwrap();
            if matches!(session.phase, super::Phase::Choosing) {
                let options = session.options();
                assert_eq!(options[0].engines, ["Windows"]);
                assert_eq!(options[0].target.extension, "mp3");
            } else {
                panic!("System audio discovery did not finish");
            }
        }
        window
            .render_to_image()
            .unwrap()
            .save(directory.join("wheel-desktop.png"))
            .unwrap();
        window.resize(gpui::size(gpui::px(900.), gpui::px(560.)));
    }
    if step == 25 {
        window
            .render_to_image()
            .unwrap()
            .save(directory.join("wheel-laptop.png"))
            .unwrap();
        view.update(cx, |app, cx| {
            app.converter.as_mut().unwrap().expanded = true;
            cx.notify();
        });
    }
    if step == 16 {
        window.dispatch_keystroke(gpui::Keystroke::parse("right").unwrap(), cx);
        assert_eq!(view.read(cx).converter.as_ref().unwrap().selected, 1);
        window.dispatch_keystroke(gpui::Keystroke::parse("left").unwrap(), cx);
        assert_eq!(view.read(cx).converter.as_ref().unwrap().selected, 0);
    }
    if step == 38 {
        window
            .render_to_image()
            .unwrap()
            .save(directory.join("options-laptop.png"))
            .unwrap();
        if std::env::var_os("ATLAS_CONVERSION_LAB_MOUSE").is_some() {
            let bounds = view
                .read(cx)
                .converter
                .as_ref()
                .unwrap()
                .wheel_bounds
                .unwrap();
            let center = bounds.center();
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    position: gpui::point(center.x, center.y - bounds.size.height * 0.34),
                    button: gpui::MouseButton::Left,
                    modifiers: gpui::Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
        } else {
            window.dispatch_keystroke(gpui::Keystroke::parse("enter").unwrap(), cx);
        }
        assert!(matches!(
            view.read(cx).converter.as_ref().unwrap().phase,
            super::Phase::Running { .. }
        ));
    }
    if step >= 50
        && view
            .read(cx)
            .converter
            .as_ref()
            .is_some_and(|s| matches!(s.phase, super::Phase::Finished(_)))
    {
        window
            .render_to_image()
            .unwrap()
            .save(directory.join("results-laptop.png"))
            .unwrap();
        let session = view.read(cx).converter.as_ref().unwrap();
        if let super::Phase::Finished(report) = &session.phase {
            assert!(report.errors().is_empty(), "{:?}", report.errors());
            assert_eq!(report.outputs().len(), 1);
            let message = if std::env::var_os("ATLAS_CONVERSION_LAB_AUDIO").is_some() {
                assert!(std::fs::metadata(&report.outputs()[0]).unwrap().len() > 500);
                "Native window rendered at 1280x800 and 900x560. WAV to MP3 completed through Windows; no external engine was selected.\n"
            } else {
                assert!(image::open(&report.outputs()[0]).is_ok());
                "Native window rendered at 1280x800 and 900x560. JPEG conversion decoded successfully.\n"
            };
            std::fs::write(directory.join("passed.txt"), message).unwrap();
        }
        cx.quit();
        return true;
    }
    if step >= 600 {
        panic!("Conversion lab did not finish");
    }
    window.refresh();
    false
}
