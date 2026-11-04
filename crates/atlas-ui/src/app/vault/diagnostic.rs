//! Hidden native render + real controller regression, with isolated test data.
use super::{App, Phase};
use anyhow::ensure;
use gpui::{px, size, AppContext, Entity, Window, WindowBounds, WindowOptions};
use gpui_component::{Root, ThemeRegistry, TitleBar};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn draw(window: &mut Window, cx: &mut gpui::App) {
    window.simulate_next_frame(cx);
    window.refresh();
    window.draw(cx).clear(cx);
}
fn fill(app: &Entity<App>, values: &[&str], window: &mut Window, cx: &mut gpui::App) {
    app.update(cx, |app, cx| {
        app.ensure_vault_inputs(window, cx);
        let inputs = app.vault.as_ref().unwrap().inputs.clone();
        for (input, value) in inputs.iter().zip(values) {
            input.update(cx, |input, cx| {
                input.set_value((*value).to_owned(), window, cx)
            });
        }
    });
}
fn capture(
    app: &Entity<App>,
    directory: &std::path::Path,
    label: &str,
    window: &mut Window,
    cx: &mut gpui::App,
) -> anyhow::Result<()> {
    app.update(cx, |_, cx| cx.notify());
    draw(window, cx);
    draw(window, cx);
    window
        .render_to_image()?
        .save(directory.join(format!("{label}.png")))?;
    Ok(())
}
fn idle(app: &Entity<App>, window: &mut Window, cx: &mut gpui::App) -> bool {
    app.update(cx, |app, cx| {
        app.drain_bg_results(cx);
        app.poll_vault();
    });
    draw(window, cx);
    app.read(cx).vault.as_ref().is_none_or(|d| !d.busy)
}

#[cfg(windows)]
pub fn run(directory: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    let directory = directory.canonicalize()?;
    let config = directory.join("config");
    std::fs::create_dir_all(&config)?;
    std::env::set_var("ATLAS_CONFIG_DIR", &config);
    let fixtures = directory.join("fixtures");
    std::fs::create_dir_all(&fixtures)?;
    let source = fixtures.join("Documentos personales");
    std::fs::create_dir_all(source.join("Contratos"))?;
    std::fs::write(
        source.join("Notas.txt"),
        "Documentos privados de ejemplo.\nSolo se descifran en memoria.",
    )?;
    image::RgbaImage::from_fn(480, 300, |x, y| {
        image::Rgba([(x * 255 / 480) as u8, (y * 255 / 300) as u8, 160, 255])
    })
    .save(source.join("Imagen privada.png"))?;
    let mut profiles = atlas_model::profiles::ProfileManager::default();
    profiles.profiles[0].name = "Personal".into();
    profiles.profiles[0].theme = "Atlas Light".into();
    profiles
        .profiles
        .push(atlas_model::profiles::Profile::new("Trabajo"));
    std::fs::write(
        config.join("profiles.json"),
        serde_json::to_vec_pretty(&profiles)?,
    )?;
    rust_i18n::set_locale("es");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let rt = runtime.handle().clone();
    let result = std::rc::Rc::new(std::cell::RefCell::new(Ok(())));
    let final_result = result.clone();
    gpui_platform::application().with_assets(crate::assets::AtlasAssets).run(move |cx| {
        gpui_component::init(cx);
        cx.text_system().add_fonts(crate::assets::bundled_fonts()).unwrap();
        ThemeRegistry::global_mut(cx).load_themes_from_str(include_str!("../../../assets/themes/atlas-themes.json")).unwrap();
        let mut app = None;
        let window = cx.open_window(WindowOptions { show: false, focus: false,
            titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::centered(size(px(1100.), px(780.)), cx)), ..Default::default()
        }, |window, cx| {
            let view = cx.new(|cx| App::new(rt, atlas_model::settings::Settings { disable_animations: true,
                autosave_session: false, getting_started_seen: true, show_inspector: false, ..Default::default()
            }, Instant::now(), vec![fixtures], window, cx));
            app = Some(view.clone()); cx.new(|cx| Root::new(view, window, cx))
        }).unwrap();
        let app = app.unwrap(); let window: gpui::AnyWindowHandle = window.into();
        cx.spawn(async move |cx| {
            let checked: anyhow::Result<()> = async {
                window.update(cx, |_, window, _| window.resize(size(px(1100.), px(780.))))?;
                for _ in 0..10 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    window.update(cx, |_, window, cx| draw(window, cx))?;
                }
                for (label, theme, zoom) in [("create-light", "Atlas Light", 1.0), ("create-dark", "Atlas Dracula", 1.0), ("create-zoom", "Atlas Light", 1.5)] {
                    window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            app.set_theme(theme.into()); app.state.settings.global_zoom = zoom; app.apply_active_theme(window, cx);
                            app.request_vault(source.clone(), Phase::Create); cx.notify();
                        }); capture(&app, &directory, label, window, cx)
                    })??;
                }
                window.update(cx, |_, window, cx| {
                    app.update(cx, |app, cx| { app.state.settings.global_zoom = 1.; app.apply_active_theme(window, cx); });
                    fill(&app, &["twelve paper cranes", "different password"], window, cx);
                    app.update(cx, |app, cx| app.submit_vault(window, cx));
                    ensure!(app.read(cx).vault.as_ref().unwrap().error.is_some(), "mismatch was accepted");
                    capture(&app, &directory, "password-error", window, cx)?;
                    fill(&app, &["twelve paper cranes", "twelve paper cranes"], window, cx);
                    app.update(cx, |app, cx| app.submit_vault(window, cx));
                    ensure!(app.read(cx).vault.as_ref().unwrap().inputs.is_empty(), "password entities retained during encryption");
                    Ok::<_, anyhow::Error>(())
                })??;
                let mut done = false;
                for _ in 0..200 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    done = window.update(cx, |_, w, cx| idle(&app, w, cx))?;
                    if done { break; }
                }
                ensure!(done, "encryption timed out");

                // Unlock must enter the same normal pane, with no private browser overlay.
                for _ in 0..80 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    if window.update(cx, |_, w, cx| {
                        idle(&app,w,cx);
                        app.read(cx).state.active_tab_state().is_some_and(|tab| tab.entries.iter().any(|e| e.name == "Notas.txt"))
                    })? { break; }
                }
                let (root, mount_path) = window.update(cx, |_, w, cx| {
                    let state = app.read(cx);
                    ensure!(state.vault.is_none(), "unlock left a dialog: {:?}", state.vault.as_ref().and_then(|d| d.error.clone()));
                    let mount = state.vault_mount.as_ref().ok_or_else(|| anyhow::anyhow!("native mount missing"))?;
                    let root = mount.root().to_path_buf(); let path = mount.path().to_path_buf();
                    ensure!(state.state.active_tab_data().unwrap().location.fs_dir() == Some(path.as_path()), "unlock changed the wrong pane");
                    ensure!(!source.exists(), "plaintext originals remain");
                    capture(&app, &directory, "unlocked-tab", w, cx)?;
                    app.update(cx, |app, cx| {
                        app.state.settings.show_inspector = true;
                        let tab = app.state.active_tab_state_mut().unwrap();
                        let index = tab.entries.iter().position(|e| e.name == "Imagen privada.png").unwrap();
                        tab.selection.selected.clear(); tab.selection.selected.insert(index);
                        app.dirty_all_panes = true; cx.notify();
                    });
                    Ok::<_, anyhow::Error>((root,path))
                })??;
                for _ in 0..100 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    if window.update(cx, |_, w, cx| {
                        idle(&app,w,cx);
                        let state=app.read(cx);
                        matches!(state.state.preview.results.get(&state.state.focused_tile), Some(atlas_model::preview_cache::PreviewResult::Image { .. }))
                    })? { break; }
                }
                window.update(cx, |_, w, cx| {
                    let state=app.read(cx);
                    ensure!(matches!(state.state.preview.results.get(&state.state.focused_tile), Some(atlas_model::preview_cache::PreviewResult::Image { .. })), "normal inspector image preview missing");
                    capture(&app, &directory, "native-image-preview", w, cx)?;
                    // Saving while unlocked must sanitize paths before writing JSON.
                    app.update(cx, |app, _| { app.save(); });
                    let config = directory.join("config");
                    for filename in ["session.json", "profiles.json"] {
                        if let Ok(text)=std::fs::read_to_string(config.join(filename)) {
                            ensure!(!text.contains("atlas-private-mounts"), "private path persisted in {filename}");
                            ensure!(!text.contains("Imagen privada.png"), "private filename persisted in {filename}");
                        }
                    }
                    // User lock cannot revoke a file that another app still has open.
                    let file=std::fs::File::open(mount_path.join("Notas.txt"))?;
                    app.update(cx, |app, _| { assert!(!app.close_native_vault(false)); });
                    ensure!(app.read(cx).vault_mount.is_some(), "open file mount was revoked");
                    app.update(cx, |app, _| app.switch_profile(1));
                    ensure!(app.read(cx).state.profiles.active_idx == 0, "profile changed while a file was open");
                    drop(file);
                    Ok::<_, anyhow::Error>(())
                })??;
                for _ in 0..20 { cx.background_executor().timer(Duration::from_millis(50)).await; }
                window.update(cx, |_, w, cx| {
                    app.update(cx, |app, cx| { app.manage_native_vault(w,cx); });
                    ensure!(app.read(cx).vault.as_ref().is_some_and(|d| d.phase == Phase::Manage), "manage did not preserve session");
                    capture(&app, &directory, "manage-access", w, cx)?;
                    // Back from manage remounts into the pane without a second password.
                    app.update(cx, |app, cx| app.vault_phase(Phase::Browse,w,cx));
                    Ok::<_, anyhow::Error>(())
                })??;
                for _ in 0..200 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    if window.update(cx, |_,w,cx| idle(&app,w,cx))? { break; }
                }
                window.update(cx, |_, w, cx| {
                    ensure!(app.read(cx).vault_mount.is_some(), "cancel manage did not remount");
                    app.update(cx, |app, _| { assert!(app.close_native_vault(false)); });
                    ensure!(!mount_path.exists(), "old mount remains after lock");
                    app.update(cx, |app, cx| {
                        use atlas_model::file_entry::{FileEntry, FileEntryData, FileKind};
                        let entries=app.private_folder_entries(vec![FileEntry::from(FileEntryData {
                            name: root.file_name().unwrap().to_string_lossy().as_ref().into(), path: root.clone(),
                            kind: FileKind::Dir, size: None, modified: None, _is_hidden: false,
                            _is_symlink: false, archive_root: None, recycle_info: None,
                        })]);
                        let tab=app.state.active_tab_state_mut().unwrap(); tab.set_entries(entries.into());
                        tab.load_state=atlas_model::tab::LoadState::Loaded; tab.selection.selected.insert(0);
                        app.dirty_all_panes=true; cx.notify();
                    });
                    capture(&app,&directory,"locked-preview",w,cx)?;
                    app.update(cx, |app,_| { assert!(app.try_open_vault(&root)); });
                    capture(&app,&directory,"unlock",w,cx)?;
                    app.update(cx, |app,cx| { app.set_theme("Atlas Dracula".into()); app.apply_active_theme(w,cx); });
                    capture(&app,&directory,"unlock-dark",w,cx)?;
                    app.update(cx, |app,cx| { app.set_theme("Atlas Light".into()); app.apply_active_theme(w,cx); });
                    fill(&app,&["wrong password"],w,cx);
                    app.update(cx, |app,cx| app.submit_vault(w,cx));
                    Ok::<_,anyhow::Error>(())
                })??;
                for _ in 0..200 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    if window.update(cx, |_,w,cx| idle(&app,w,cx))? { break; }
                }
                window.update(cx, |_,w,cx| {
                    ensure!(app.read(cx).vault.as_ref().is_some_and(|d| d.error.is_some()), "incorrect password accepted");
                    capture(&app,&directory,"unlock-error",w,cx)?;
                    fill(&app,&["twelve paper cranes"],w,cx);
                    app.update(cx, |app,cx| app.submit_vault(w,cx));
                    Ok::<_,anyhow::Error>(())
                })??;
                for _ in 0..200 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    if window.update(cx, |_,w,cx| idle(&app,w,cx))? { break; }
                }
                let mounted = window.update(cx, |_,w,cx| {
                    let mounted = app.read(cx).vault_mount.as_ref().unwrap().path().to_path_buf();
                    app.update(cx, |app,_| app.switch_profile(1));
                    ensure!(app.read(cx).vault_mount.is_none() && app.read(cx).vault.is_none(), "profile retained session");
                    ensure!(!mounted.exists(), "mount remains visible after profile switch");
                    ensure!(!atlas_core::vault::visible_to(&root,&app.read(cx).state.profiles.active().id), "other profile sees folder");
                    app.update(cx, |app,_| { app.switch_profile(0); app.request_vault(root.clone(),Phase::Unlock); });
                    fill(&app,&["twelve paper cranes"],w,cx);
                    app.update(cx, |app,cx| app.submit_vault(w,cx));
                    Ok::<_,anyhow::Error>(mounted)
                })??;
                for _ in 0..200 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    if window.update(cx, |_,w,cx| idle(&app,w,cx))? { break; }
                }
                window.update(cx, |_,w,cx| {
                    app.update(cx, |app,cx| { app.manage_native_vault(w,cx); app.vault_phase(Phase::Restore,w,cx); });
                    capture(&app,&directory,"remove-protection",w,cx)?;
                    app.update(cx, |app,cx| app.submit_vault(w,cx));
                    Ok::<_,anyhow::Error>(())
                })??;
                for _ in 0..200 {
                    cx.background_executor().timer(Duration::from_millis(50)).await;
                    if window.update(cx, |_,w,cx| idle(&app,w,cx))? { break; }
                }
                ensure!(source.join("Notas.txt").is_file(),"restore failed");
                ensure!(!root.exists() && !mounted.exists(),"private storage remained after restore");
                std::fs::write(directory.join("passed.txt"),"Native light/dark/zoom; confirmation mismatch; encryption; same-tab navigation; inspector image preview; sanitized session and profile saves; open-file lock protection; manage remount; locked inspector; wrong password; profile unmount and isolation; restore: passed.\n")?;
                Ok(())
            }.await;
            if let Err(error) = &checked { let _ = std::fs::write(directory.join("failed.txt"), format!("{error:#}")); }
            *result.borrow_mut() = checked; cx.update(|cx| cx.quit());
        }).detach();
    });
    let final_value = std::mem::replace(&mut *final_result.borrow_mut(), Ok(()));
    final_value
}

#[cfg(not(windows))]
pub fn run(_: PathBuf) -> anyhow::Result<()> {
    anyhow::bail!("Windows native mount diagnostic")
}
