//! Real GPUI and hidden GPU captures with an isolated profile. Map captures use
//! synthetic data. With windows-cleanup enabled, execution tests touch only the
//! files this harness creates in a unique directory under the trusted user Temp.
use super::*;
use crate::app::App as AtlasApp;
use anyhow::ensure;
use atlas_model::cleanup::{DiskEntry, ScanStatus};
use gpui::*;
use gpui_component::{Root, ThemeRegistry, TitleBar};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn draw(window: &mut Window, cx: &mut gpui::App) {
    window.simulate_next_frame(cx);
    window.refresh();
    window.draw(cx).clear(cx);
}
fn fixture(panel: &mut CleanupPanel) {
    panel.job = None;
    panel.generation += 1;
    panel.hovered = None;
    let entries = [
        ("Projects", 47_600_000_000, true),
        ("Photos", 10_900_000_000, true),
        ("FinalCut_assets.zip", 11_100_000_000, false),
        ("4K Textures", 9_400_000_000, true),
        ("Recordings", 6_100_000_000, true),
        ("Downloads", 4_600_000_000, true),
        ("Music", 2_200_000_000, true),
        ("Documents", 1_700_000_000, true),
        ("Archive", 900_000_000, true),
    ]
    .into_iter()
    .map(|(name, bytes, directory)| DiskEntry {
        path: panel.volume.join(name),
        bytes,
        directory,
    })
    .collect::<Vec<_>>();
    let mut entries = entries;
    entries.sort_by_key(|e| std::cmp::Reverse(e.bytes));
    panel.snapshot = Arc::new(DiskSnapshot {
        generation: panel.generation,
        root: panel.volume.clone(),
        status: ScanStatus::Partial,
        bytes: entries.iter().map(|e| e.bytes).sum(),
        entry_count: 324_812,
        skipped: 12,
        capacity: Some((512_000_000_000, 97_400_000_000)),
        fast_scan_available: true,
        large_files: entries.iter().filter(|e| !e.directory).cloned().collect(),
        entries,
        ..Default::default()
    });
    panel.rebuild();
}

fn save(
    directory: &Path,
    name: &str,
    window: &mut Window,
    cx: &mut gpui::App,
) -> anyhow::Result<()> {
    draw(window, cx);
    window.render_to_image()?.save(directory.join(name))?;
    Ok(())
}

#[cfg(all(windows, feature = "windows-cleanup"))]
struct ExecutionFixture(PathBuf);

#[cfg(all(windows, feature = "windows-cleanup"))]
impl ExecutionFixture {
    fn new() -> anyhow::Result<Self> {
        let temp = atlas_core::cleanup::user_temp_root()
            .ok_or_else(|| anyhow::anyhow!("No trusted user Temp folder"))?;
        let root = temp.join(format!(
            "atlas-cleanup-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir(&root)?;
        let fixture = Self(root);
        for name in ["reviewed.tmp", "changed.tmp", "unselected.tmp"] {
            let path = fixture.0.join(name);
            std::fs::write(&path, b"laboratory temporary data")?;
            std::fs::File::options()
                .write(true)
                .open(path)?
                .set_modified(std::time::SystemTime::now() - Duration::from_secs(8 * 24 * 3600))?;
        }
        Ok(fixture)
    }
}

#[cfg(all(windows, feature = "windows-cleanup"))]
impl Drop for ExecutionFixture {
    fn drop(&mut self) {
        // Explicit leaves only; never recursively remove a Temp directory.
        for name in ["reviewed.tmp", "changed.tmp", "unselected.tmp"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

pub fn run(directory: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    let directory = directory.canonicalize()?;
    let directory = PathBuf::from(directory.to_string_lossy().trim_start_matches(r"\\?\"));
    let config = directory.join("config");
    std::fs::create_dir_all(&config)?;
    std::env::set_var("ATLAS_CONFIG_DIR", &config);
    std::env::set_var("ATLAS_DISABLE_SEARCH_SERVICE", "1");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let rt = runtime.handle().clone();
    let outcome = Rc::new(RefCell::new(Ok(())));
    let final_outcome = outcome.clone();
    gpui_platform::application().with_assets(crate::assets::AtlasAssets).run(move |cx| {
        gpui_component::init(cx);cx.text_system().add_fonts(crate::assets::bundled_fonts()).unwrap();
        ThemeRegistry::global_mut(cx).load_themes_from_str(include_str!("../../../assets/themes/atlas-themes.json")).unwrap();
        rust_i18n::set_locale("es");
        let mut app=None;
        let handle=cx.open_window(WindowOptions {show:false,focus:false,titlebar:Some(TitleBar::title_bar_options()),
            window_bounds:Some(WindowBounds::centered(size(px(1280.),px(850.)),cx)),..Default::default()},|window,cx| {
                let view=cx.new(|cx|AtlasApp::new(rt,atlas_model::settings::Settings {disable_animations:true,autosave_session:false,
                    getting_started_seen:true,show_inspector:false,..Default::default()},Instant::now(),vec![directory.clone()],window,cx));
                app=Some(view.clone());cx.new(|cx|Root::new(view,window,cx))
            }).unwrap();
        let app=app.unwrap();let window:AnyWindowHandle=handle.into();
        cx.spawn(async move |cx| {
            let checked:anyhow::Result<()>=async {
                let mut checks=Vec::new();
                let mut benchmarks=Vec::new();
                for (label,theme,zoom,width,height) in [("dark","Atlas Dracula",1.,1280.,850.),("light","Atlas Light",1.,1280.,850.),
                    ("laptop","Atlas Dracula",1.,900.,600.),("zoom","Atlas Dracula",1.5,1280.,900.),
                    ("windows","Atlas Windows",1.,1280.,850.),("rose","Atlas Rosé",1.,1280.,850.)] {
                    window.update(cx,|_,window,cx| {
                        window.resize(size(px(width),px(height)));
                        app.update(cx,|app,cx| {app.set_theme(theme.into());app.state.settings.global_zoom=zoom;app.apply_active_theme(window,cx);
                            let available=app.diagnostic_cleanup_menu(PathBuf::from("C:\\"),true,window,cx);
                            assert!(available,"Drive menu did not expose the cleanup action");});
                        let panel=app.read(cx).cleanup_panel.clone().unwrap();
                        panel.update(cx,|p,cx| {p.tab=Tab::Map;p.list_only=false;fixture(p);cx.notify();});draw(window,cx);
                    })?;
                    for _ in 0..6 {cx.background_executor().timer(Duration::from_millis(35)).await;window.update(cx,|_,w,cx|draw(w,cx))?;}
                    window.update(cx,|_,window,cx|->anyhow::Result<()> {
                        let panel=app.read(cx).cleanup_panel.clone().unwrap();
                        let final_snapshot=panel.read(cx).snapshot.clone();
                        panel.update(cx,|p,cx| {
                            let mut partial=(*p.snapshot).clone();partial.status=ScanStatus::Scanning;
                            p.snapshot=Arc::new(partial);p.rebuild();
                            assert!(p.map_nodes.is_empty(),"Progress changed the visible map");
                            let revision=p.revision;
                            for _ in 0..20 {p.rebuild();assert_eq!(p.revision,revision,"Progress invalidated geometry");}
                            p.snapshot=final_snapshot;p.rebuild();cx.notify();
                        });draw(window,cx);
                        save(&directory,&format!("map-{label}.png"),window,cx)?;
                        let cache=panel.read(cx).map_cache.borrow();
                        ensure!(!cache.circles.is_empty(),"Map was blank: {:?}, key {}, nodes {}",cache.bounds,cache.key,panel.read(cx).map_nodes.len());
                        let circle=cache.circles[0];let key=cache.key.clone();let bounds=cache.bounds;let rebuilds=cache.rebuilds;drop(cache);
                        ensure!(circle.radius>0.);save(&directory,&format!("map-{label}.png"),window,cx)?;
                        draw(window,cx);ensure!(panel.read(cx).map_cache.borrow().key==key,"Stable map rebuilt its geometry");
                        ensure!(panel.read(cx).map_cache.borrow().rebuilds==rebuilds,"Idle redraw recomputed packing");
                        let mut frames=Vec::new();
                        for _ in 0..20 {let start=Instant::now();draw(window,cx);frames.push(start.elapsed().as_secs_f64()*1000.);}
                        frames.sort_by(f64::total_cmp);
                        benchmarks.push(serde_json::json!({"name":format!("{label}: cached fixture redraw (debug, CPU wall time)"),
                            "metrics":{"median_ms":frames[10],"p95_ms":frames[18]}}));
                        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent {position:point(bounds.origin.x+px(circle.x),bounds.origin.y+px(circle.y)),
                            button:MouseButton::Left,modifiers:Default::default(),click_count:1,first_mouse:false}),cx);
                        draw(window,cx);
                        ensure!(panel.read(cx).selected.is_some(),"Circular hit testing did not select an entry");
                        window.dispatch_keystroke(Keystroke::parse("down")?,cx);draw(window,cx);
                        ensure!(panel.read(cx).selected.is_some(),"Keyboard did not select a row");
                        let focus=panel.read(cx).focus.clone();window.focus(&focus,cx);draw(window,cx);
                        let before=panel.read(cx).selected.clone();let mut moved=false;
                        for k in ["right","left","up","down","right"] {
                            window.dispatch_keystroke(Keystroke::parse(k)?,cx);draw(window,cx);
                            if panel.read(cx).selected!=before {moved=true;}
                        }
                        ensure!(moved,"Arrow keys did not move between bubbles");
                        panel.update(cx,|p,cx|{p.list_only=true;cx.notify();});
                        save(&directory,&format!("list-{label}.png"),window,cx)?;
                        window.dispatch_keystroke(Keystroke::parse("escape")?,cx);draw(window,cx);draw(window,cx);
                        Ok(())
                    })??;
                    // The panel closes through a deferred window effect.
                    window.update(cx,|_,window,cx|->anyhow::Result<()> {
                        draw(window,cx);
                        ensure!(app.read(cx).cleanup_panel.is_none(),"Escape did not dismiss the panel ({label}, {})", app.read(cx).cleanup_panel.is_some());
                        ensure!(app.read(cx).focus_handle.is_focused(window),"Focus not restored");
                        Ok(())
                    })??;
                    checks.push(serde_json::json!({"name":format!("{label}: map, list, keyboard, focus, stable geometry"),"status":"passed"}));
                }
                window.update(cx,|_,window,cx|->anyhow::Result<()> {
                    app.update(cx,|app,cx| {
                        assert!(!app.diagnostic_cleanup_menu(directory.join("ordinary-file.txt"),false,window,cx),"Cleanup leaked into file menus");
                        app.state.settings.global_zoom=1.;app.apply_active_theme(window,cx);app.open_disk_cleanup(PathBuf::from("C:\\"),cx);
                    });
                    window.resize(size(px(1280.),px(850.)));
                    let panel=app.read(cx).cleanup_panel.clone().unwrap();
                    panel.update(cx,|p,cx| {
                        p.job=None;p.generation+=1;p.tab=Tab::Cleanup;p.checked.clear();
                        // Synthetic paths: no dependency on Known Folder access
                        // and no discovery of the user's actual temporary data.
                        let temp=PathBuf::from(r"C:\Users\Example\AppData\Local\Temp");
                        let candidates=(0..4).map(|i|atlas_model::cleanup::CleanupCandidate {
                            path:temp.join(format!("atlas-diagnostic-{i}.tmp")),identity:atlas_model::cleanup::FileIdentity {
                                volume:0,id:[i;16],bytes:2_400_000,modified:0,links:1},ancestors:Vec::new()
                        }).collect();
                        p.snapshot=Arc::new(DiskSnapshot {generation:p.generation,root:temp,status:ScanStatus::Complete,candidates,..Default::default()});
                        p.rebuild();cx.notify();
                    });
                    draw(window,cx);window.dispatch_keystroke(Keystroke::parse("space")?,cx);draw(window,cx);
                    ensure!(panel.read(cx).checked.len()==1,"Space did not toggle the temporary candidate");
                    save(&directory,"cleanup-selection.png",window,cx)?;
                    panel.update(cx,|p,cx| {
                        p.review=Some(CleanupPlan {version:1,id:"synthetic-not-executable".into(),created_unix_seconds:0,
                            volume_root:p.volume.clone(),temp_root:p.snapshot.root.clone(),candidates:p.snapshot.candidates.iter().filter(|c|p.checked.contains(&c.path)).cloned().collect()});cx.notify();
                    });
                    save(&directory,"cleanup-review.png",window,cx)?;
                    window.dispatch_keystroke(Keystroke::parse("escape")?,cx);draw(window,cx);draw(window,cx);
                    ensure!(panel.read(cx).review.is_none(),"Escape did not return from review");
                    ensure!(app.read(cx).cleanup_panel.is_some(),"Review Escape closed the whole panel");
                    panel.update(cx,|p,cx| {
                        let snap=|n:usize,b:u64|Some(Arc::new(DiskSnapshot {status:ScanStatus::Complete,
                            reclaimable:(0..n).map(|i|atlas_model::cleanup::DiskEntry {path:PathBuf::from(format!("x{i}")),bytes:b,directory:false}).collect(),..Default::default()}));
                        p.cats=vec![
                            super::CatState {key:"cleanup_cat_temp",roots:vec![PathBuf::new()],snaps:vec![snap(8000,90_000)],enabled:true},
                            super::CatState {key:"cleanup_cat_browser",roots:vec![PathBuf::new()],snaps:vec![snap(3000,420_000)],enabled:true},
                            super::CatState {key:"cleanup_cat_crash",roots:vec![PathBuf::new()],snaps:vec![snap(12,3_000_000)],enabled:false},
                            super::CatState {key:"cleanup_cat_dev",roots:vec![PathBuf::new()],snaps:vec![snap(900,2_900_000)],enabled:true},
                        ];
                        p.cat_pending=0;p.tab=Tab::Cleanup;cx.notify();
                    });
                    draw(window,cx);
                    save(&directory,"cleanup-categories.png",window,cx)?;
                    panel.update(cx,|p,cx| {p.cats.clear();cx.notify();});
                    window.dispatch_keystroke(Keystroke::parse("escape")?,cx);draw(window,cx);draw(window,cx);
                    Ok(())
                })??;
                checks.push(serde_json::json!({"name":"Drive-only menu, temporary selection, permanent preview, nested Escape; no execution","status":"passed"}));
                #[cfg(all(windows, feature = "windows-cleanup"))]
                {
                    let fixture = ExecutionFixture::new()?;
                    let candidates = ["reviewed.tmp", "changed.tmp", "unselected.tmp"]
                        .map(|name| atlas_win_space::cleanup::inspect(&fixture.0.join(name)))
                        .into_iter().collect::<std::io::Result<Vec<_>>>()?;
                    let volume = fixture.0.ancestors().last().unwrap().to_owned();
                    let panel = window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| app.open_disk_cleanup(volume, cx));
                        let panel = app.read(cx).cleanup_panel.clone().unwrap();
                        panel.update(cx, |p, cx| {
                            p.job = None;
                            p.generation += 1;
                            p.tab = Tab::Cleanup;
                            p.checked.clear();
                            p.excluded.insert(candidates[2].path.clone());
                            p.snapshot = Arc::new(DiskSnapshot {
                                generation: p.generation,
                                root: atlas_core::cleanup::user_temp_root().unwrap(),
                                status: ScanStatus::Complete,
                                candidates,
                                ..Default::default()
                            });
                            p.rebuild();
                            p.review_selection(cx);
                        });
                        draw(window, cx);
                        panel
                    })?;
                    let deadline = Instant::now() + Duration::from_secs(10);
                    loop {
                        cx.background_executor().timer(Duration::from_millis(20)).await;
                        let preparing = window.update(cx, |_, window, cx| {
                            draw(window, cx);
                            panel.read(cx).preparing
                        })?;
                        if !preparing { break; }
                        ensure!(Instant::now() < deadline, "Timed out preparing reviewed plan");
                    }
                    let reviewed = window.update(cx, |_, window, cx| -> anyhow::Result<_> {
                        let plan = panel.read(cx).review.clone()
                            .ok_or_else(|| anyhow::anyhow!("Preparation failed: {:?}", panel.read(cx).error))?;
                        ensure!(plan.candidates.len() == 2, "Unselected file entered the plan");
                        ensure!(config.join("cleanup").join(format!("{}.plan.json", plan.id)).is_file());
                        save(&directory, "cleanup-native-review.png", window, cx)?;
                        // Simulate an application updating a candidate after review.
                        std::fs::write(fixture.0.join("changed.tmp"), b"modified after review")?;
                        panel.update(cx, |p, cx| p.execute(cx));
                        Ok(plan)
                    })??;
                    let deadline = Instant::now() + Duration::from_secs(10);
                    loop {
                        cx.background_executor().timer(Duration::from_millis(20)).await;
                        let executing = window.update(cx, |_, window, cx| {
                            draw(window, cx);
                            panel.read(cx).execution.is_some()
                        })?;
                        if !executing { break; }
                        ensure!(Instant::now() < deadline, "Timed out executing fixture plan");
                    }
                    window.update(cx, |_, window, cx| -> anyhow::Result<()> {
                        let state = panel.read(cx);
                        ensure!(state.tab == Tab::History, "Execution did not open results");
                        let report = state.report.as_ref()
                            .ok_or_else(|| anyhow::anyhow!("No report: {:?}", state.error))?;
                        ensure!(report.error.is_none(), "Persistence failed: {:?}", report.error);
                        ensure!(report.results.len() == 2);
                        ensure!(report.results.iter().filter(|r| r.error.is_none()).count() == 1);
                        ensure!(!fixture.0.join("reviewed.tmp").exists());
                        ensure!(std::fs::read(fixture.0.join("changed.tmp"))? == b"modified after review");
                        ensure!(std::fs::read(fixture.0.join("unselected.tmp"))? == b"laboratory temporary data");
                        let persisted: CleanupReport = serde_json::from_slice(&std::fs::read(
                            config.join("cleanup").join(format!("{}.report.json", reviewed.id)))?)?;
                        ensure!(persisted.results == report.results);
                        save(&directory, "cleanup-native-results.png", window, cx)?;
                        Ok(())
                    })??;
                    let replay = cx.background_executor().spawn(async move {
                        atlas_core::cleanup::execute(&reviewed, &AtomicBool::new(false), |_, _| {})
                    }).await;
                    ensure!(replay.is_err(), "Consumed plan could be replayed");
                    checks.push(serde_json::json!({"name":"Native UI prepare/execute/results: selected old file deleted, changed and unselected files preserved, report persisted, replay rejected","status":"passed"}));
                }
                // Drain the native checkbox's short-lived keyed-state tasks
                // before tearing down GPUI (its transition duration is 250 ms).
                cx.background_executor().timer(Duration::from_millis(300)).await;
                window.update(cx, |_, window, cx| draw(window, cx))?;
                std::fs::write(directory.join("checks.json"),serde_json::to_vec_pretty(&checks)?)?;
                std::fs::write(directory.join("benchmarks.json"),serde_json::to_vec_pretty(&benchmarks)?)?;
                Ok(())
            }.await;
            if let Err(error)=&checked {let _=std::fs::write(directory.join("failed.txt"),format!("{error:#}"));}
            *outcome.borrow_mut()=checked;let _=cx.update(|cx|cx.quit());
        }).detach();
    });
    let result = std::mem::replace(&mut *final_outcome.borrow_mut(), Ok(()));
    result
}
